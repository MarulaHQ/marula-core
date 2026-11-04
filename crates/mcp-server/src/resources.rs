//! MCP resources: read-only records addressed by URI.
//!
//! A resource is not a new permission. Each family below names the catalog
//! tool it reuses, so the URI a caller reads resolves to the same path, the
//! same capability and the same proxied read as the equivalent tool call.
//! Authorization is therefore identical on both surfaces, and there is no
//! second table to keep in step with the OpenAPI snapshot.
//!
//! A resource is an instance, so only URI templates are published
//! (`resources/templates/list`). The adapter does not enumerate an
//! organization's tickets, clients or documents: that is a paginated,
//! RLS-scoped query, and the tool catalog already answers it. `resources/list`
//! is empty by design, not by omission.

/// URI scheme and authority for every Marula record.
pub const SCHEME: &str = "marula://";

/// One record family exposed as a resource template.
#[derive(Debug)]
pub struct Family {
    /// URI authority: `marula://tickets/{id}`.
    pub family: &'static str,
    /// The catalog tool this family reuses for path, capability and proxy.
    pub tool: &'static str,
    pub title: &'static str,
    pub description: &'static str,
}

/// The record families the surface publishes. Kept short on purpose: each
/// entry costs a real read path, and these three are the records an agent
/// reaches for first.
pub const FAMILIES: &[Family] = &[
    Family {
        family: "tickets",
        tool: "tickets_get_one",
        title: "Ticket",
        description: "One service ticket, as its detail view returns it.",
    },
    Family {
        family: "clients",
        tool: "clients_get_one",
        title: "Client",
        description: "One client organization, as its detail view returns it.",
    },
    Family {
        family: "documents",
        tool: "documents_get",
        title: "Document",
        description: "One document record and its current version.",
    },
];

/// `marula://<family>/{id}` for one family.
pub fn uri_template(family: &Family) -> String {
    format!("{SCHEME}{}/{{id}}", family.family)
}

/// The concrete URI for one record.
pub fn uri_for(family: &Family, id: &str) -> String {
    format!("{SCHEME}{}/{id}", family.family)
}

/// A parsed resource URI: which family, and which record.
#[derive(Debug)]
pub struct Reference<'a> {
    pub family: &'a Family,
    pub id: String,
}

/// Parses `marula://<family>/<id>`.
///
/// Deliberately strict. The id is taken literally: a percent escape, a query,
/// a fragment or a second path segment is refused rather than decoded, so no
/// caller can reshape the proxied path. The identifier grammar is the one
/// every record id in this API uses — ASCII alphanumerics, `-` and `_` — which
/// also rules out `.` and `..` as a segment.
pub fn parse(uri: &str) -> Result<Reference<'static>, String> {
    let rest = uri
        .strip_prefix(SCHEME)
        .ok_or_else(|| format!("unknown resource URI: expected {SCHEME}<family>/<id>"))?;
    if rest.contains(['?', '#', '%']) {
        return Err("resource URI carries a query, fragment or escape; pass the id plainly".into());
    }
    let (family, id) = rest
        .split_once('/')
        .ok_or_else(|| format!("unknown resource URI: expected {SCHEME}<family>/<id>"))?;
    let family = FAMILIES
        .iter()
        .find(|candidate| candidate.family == family)
        .ok_or_else(|| {
            let known: Vec<&str> = FAMILIES.iter().map(|f| f.family).collect();
            format!(
                "unknown resource family; this server publishes {}",
                known.join(", ")
            )
        })?;
    if !is_record_id(id) {
        return Err("resource id must be 1..=64 ASCII alphanumerics, '-' or '_'".into());
    }
    Ok(Reference {
        family,
        id: id.to_string(),
    })
}

/// Whether `value` is a plausible record identifier. Used for both a resource
/// URI's last segment and a prompt argument that names a record, so the two
/// surfaces accept exactly the same grammar.
pub fn is_record_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_family_parses() {
        let reference = parse("marula://tickets/018f0a5e-0000-7000-8000-000000000001").unwrap();
        assert_eq!(reference.family.family, "tickets");
        assert_eq!(reference.id, "018f0a5e-0000-7000-8000-000000000001");
    }

    #[test]
    fn every_published_family_parses() {
        for family in FAMILIES {
            let uri = uri_for(family, "abc123");
            let reference = parse(&uri).unwrap_or_else(|error| panic!("{uri}: {error}"));
            assert_eq!(reference.family.tool, family.tool);
        }
    }

    #[test]
    fn a_template_and_a_concrete_uri_agree() {
        let family = &FAMILIES[0];
        assert_eq!(uri_template(family), "marula://tickets/{id}");
        assert_eq!(uri_for(family, "abc"), "marula://tickets/abc");
    }

    #[test]
    fn another_scheme_is_refused() {
        let error = parse("file:///etc/passwd").unwrap_err();
        assert!(error.contains("marula://"), "{error}");
    }

    #[test]
    fn an_unknown_family_is_refused_and_the_known_ones_are_named() {
        let error = parse("marula://invoices/abc").unwrap_err();
        assert!(error.contains("tickets"), "{error}");
    }

    /// The id cannot add a segment, open a query, close the path or smuggle an
    /// escape past the adapter.
    #[test]
    fn a_reshaping_id_is_refused() {
        for uri in [
            "marula://tickets/../audit-events",
            "marula://tickets/a/b",
            "marula://tickets/abc?admin=1",
            "marula://tickets/abc#frag",
            "marula://tickets/%2e%2e%2fadmin",
            "marula://tickets/",
            "marula://tickets",
            "marula://tickets/abc def",
            "marula://tickets/..",
        ] {
            assert!(parse(uri).is_err(), "{uri} must be refused");
        }
    }

    #[test]
    fn the_id_grammar_is_bounded() {
        assert!(is_record_id("abc"));
        assert!(is_record_id("018f0a5e-0000-7000-8000-000000000001"));
        assert!(!is_record_id(""));
        assert!(!is_record_id("a b"));
        assert!(!is_record_id("a/b"));
        assert!(!is_record_id(&"a".repeat(65)));
    }
}
