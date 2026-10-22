//! `http_request` across BOTH isolation transports, with no database: the guest
//! asks, the worker forwards a frame, the service answers, the guest reads the
//! answer (Wave 53 lane 12, phase 2).
//!
//! Why this file exists: the composed credential test
//! (`tests/http_credential_flow.rs`) needs Postgres and a `Recorder` transport.
//! This one needs neither, and it is the one that proves the transport claim the
//! ADR makes about `in_process`: the same guest, the same service and the same
//! bytes, whether the frame went through a pipe or through a channel inside
//! core-api. A plugin that works in one mode and not the other is a plugin whose
//! author cannot test what they ship.
//!
//! The service here is a double that answers a fixed response — the point is the
//! PLUMBING (verb 5 out, an `Http` value back), not the store or the vault.

use plugin_host::abi::{HttpResponse, InvocationOutcome, InvocationRequest};
use plugin_host::isolation::{ModuleArtifact, SandboxLimits};
use plugin_host::runtime::{
    HostCallContext, HostCallService, IsolationMode, PluginPool, PoolConfig,
};
use plugin_host::wire;
use uuid::Uuid;

fn pool_config(mode: IsolationMode) -> PoolConfig {
    PoolConfig {
        mode,
        workers: 1,
        worker_bin: Some(env!("CARGO_BIN_EXE_marula-plugin-worker").into()),
        health: None,
        sandbox_limits: SandboxLimits::default(),
    }
}

fn artifact_from_wat(source: &str) -> ModuleArtifact {
    let bytes = wat::parse_str(source).expect("valid WAT");
    ModuleArtifact {
        digest: ModuleArtifact::digest_of(&bytes),
        bytes,
    }
}

/// A guest that calls `http_request` under the `acme` profile and returns the
/// reply frame unchanged. Memory: profile at 0 (`acme`, 4 bytes), the JSON
/// request at 16 (49 bytes), `marula_alloc` hands out space from 1024.
fn egress_guest() -> ModuleArtifact {
    artifact_from_wat(
        r#"
        (module
          (import "marula" "http_request" (func $http (param i32 i32 i32 i32) (result i64)))
          (memory (export "memory") 1)
          (data (i32.const 0) "acme")
          (data (i32.const 16) "{\"method\":\"GET\",\"url\":\"https://api.acme.test/v1\"}")
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (call $http (i32.const 0) (i32.const 4) (i32.const 16) (i32.const 49))))
        "#,
    )
}

/// The request the parent dispatched: one consented profile, one grant.
fn request(install_id: Uuid) -> InvocationRequest {
    InvocationRequest {
        invocation_id: Uuid::now_v7(),
        org_id: Uuid::now_v7(),
        install_id,
        entry: "plugin.egress.ask".into(),
        payload: b"{}".to_vec(),
        granted: vec!["automation.reads".into()],
        declared_events: Vec::new(),
        egress: [(
            "acme".to_string(),
            vec!["https://api.acme.test/v1".to_string()],
        )]
        .into_iter()
        .collect(),
    }
}

/// What the service was asked, so the test can prove the frame carried the
/// profile and the guest's request rather than something the parent invented.
#[derive(Default)]
struct Answering {
    seen: Vec<(String, String)>,
}

impl HostCallService for Answering {
    fn serve<'a>(
        &'a mut self,
        _context: HostCallContext,
        op: wire::HostCallOp,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = wire::HostCallOutcome> + Send + 'a>>
    {
        Box::pin(async move {
            match op {
                wire::HostCallOp::HttpRequest { profile, request } => {
                    self.seen.push((profile, request.url));
                    wire::HostCallOutcome::Ok(wire::HostCallValue::Http(HttpResponse {
                        status: 200,
                        headers: Default::default(),
                        body: "{\"ok\":true}".into(),
                    }))
                }
                other => {
                    panic!("the guest asked for {other:?}, which this service does not answer")
                }
            }
        })
    }
}

#[tokio::test]
async fn the_two_transports_carry_http_request_identically() {
    let guest = egress_guest();

    let pooled = PluginPool::start(pool_config(IsolationMode::Pool))
        .await
        .expect("the pool starts");
    let mut pool_service = Answering::default();
    let in_pool = pooled
        .invoke_with(&request(Uuid::now_v7()), &guest, &mut pool_service)
        .await;
    pooled.shutdown().await;

    let in_process = PluginPool::start(pool_config(IsolationMode::InProcess))
        .await
        .expect("the in-process sandbox builds");
    let mut process_service = Answering::default();
    let in_own_process = in_process
        .invoke_with(&request(Uuid::now_v7()), &guest, &mut process_service)
        .await;

    for (label, result) in [("pool", &in_pool), ("in_process", &in_own_process)] {
        assert_eq!(
            result.reply.outcome,
            InvocationOutcome::Ok,
            "{label}: {:?}",
            result.reply
        );
    }
    assert_eq!(
        in_pool.reply.output, in_own_process.reply.output,
        "the two transports must hand the guest the same answer"
    );

    // The guest's answer is the response the service produced, framed.
    let output = in_own_process.reply.output.clone().expect("a data frame");
    let answer = plugin_host::abi::parse_frame(&output).expect("an ok frame");
    let response: HttpResponse = serde_json::from_slice(answer).expect("the response JSON");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "{\"ok\":true}");

    // And the frame carried what the guest asked for, in both modes.
    for (label, service) in [("pool", &pool_service), ("in_process", &process_service)] {
        assert_eq!(
            service.seen,
            vec![("acme".to_string(), "https://api.acme.test/v1".to_string())],
            "{label}: the frame carries the profile and the guest's URL"
        );
    }
}
