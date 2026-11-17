// The Rust engine's C interface (engine/src/ffi.rs).
#pragma once
#include <stdbool.h>

/// An event: its name and a JSON payload, valid only during the call. Called on an engine thread.
typedef void (*is_event_fn)(const char *name, const char *payload);
/// A question for the app (a JSON request); returns a JSON answer allocated with malloc, or NULL. Called on an
/// engine thread that waits for the answer.
typedef char *(*is_host_fn)(const char *request);
/// The answer to is_call: the context passed in, success, and JSON (the value, or the error as a JSON string).
typedef void (*is_reply_fn)(void *ctx, bool ok, const char *json);

bool is_start(const char *data_dir, is_event_fn on_event, is_host_fn on_host);
void is_call(const char *method, const char *args_json, void *ctx, is_reply_fn reply);
void is_stop(void);
