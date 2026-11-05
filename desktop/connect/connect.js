/*
 * The first-run screen's one job: hand the shell a decision.
 *
 * Validation lives in the Rust side (`connection::parse_server_url`) so this
 * page and `--server` cannot disagree about what an address is. This file only
 * collects a value, calls the command, and reports what came back.
 */
(function () {
  "use strict";

  var state = window.__MARULA_CONNECT__ || {};
  var form = document.getElementById("connect-form");
  var field = document.getElementById("server");
  var error = document.getElementById("server-error");
  var primary = document.getElementById("connect");
  var localButton = document.getElementById("local");
  var localBlock = document.getElementById("local-block");
  var or = document.getElementById("or");
  var notice = document.getElementById("notice");

  if (state.notice) {
    notice.textContent = state.notice;
    notice.hidden = false;
  }

  if (state.embedded) {
    localBlock.hidden = false;
    or.hidden = false;
  }

  function invoke(payload) {
    var tauri = window.__TAURI__;
    if (!tauri || !tauri.core || typeof tauri.core.invoke !== "function") {
      return Promise.reject(
        new Error("This build cannot reach the desktop shell. Start it with --server <url>.")
      );
    }
    return tauri.core.invoke("save_connection", payload);
  }

  function busy(on, label) {
    primary.disabled = on;
    localButton.disabled = on;
    field.disabled = on;
    if (label) primary.textContent = label;
  }

  function fail(message) {
    error.textContent = message;
    error.hidden = false;
    field.setAttribute("aria-invalid", "true");
    field.focus();
  }

  function clear() {
    error.textContent = "";
    error.hidden = true;
    field.removeAttribute("aria-invalid");
  }

  function submit(payload, label) {
    clear();
    busy(true, label);
    invoke(payload).then(
      function () {
        // The shell closes this window and opens the main one; there is
        // nothing left for the page to do.
      },
      function (reason) {
        busy(false, "Connect");
        fail(typeof reason === "string" ? reason : String((reason && reason.message) || reason));
      }
    );
  }

  form.addEventListener("submit", function (event) {
    event.preventDefault();
    submit({ mode: "remote", serverUrl: field.value.trim() }, "Connecting…");
  });

  localButton.addEventListener("click", function () {
    submit({ mode: "local" }, "Starting…");
  });

  field.addEventListener("input", clear);
  field.focus();
})();
