//! Sandboxed JS workflow-script engine for Forge's mesh-routed multi-agent orchestration
//! (docs/rfcs/forge-workflow.md). Domain-agnostic on purpose: this crate knows nothing about
//! Forge's mesh/subagent types — it just embeds rquickjs, lets a caller register a fixed set of
//! named async host functions (JSON in, JSON out), and runs a script that can call them via
//! `await`. `forge-core` registers the real `agent`/`pipeline`/`parallel`/`phase`/`log`/
//! `workflow` functions on top of this and owns all Forge-specific behavior.
//!
//! No ambient filesystem/network/process access is exposed to a script — only the host functions
//! the caller explicitly registers become callable globals. That's the entire sandboxing story;
//! rquickjs itself has no such access to begin with.
//!
//! The Rust<->JS value boundary is a hand-rolled `rquickjs::Value` tree-walker using only native
//! accessors (`Object`/`Array` get/set, `String::from_str`/`to_string`) — NOT the engine's own
//! `JSON.stringify`/`JSON.parse`. That looked like the obvious choice at first, but invoking a JS
//! *function* (calling `stringify`/`parse`) from inside an async host function's spawned future
//! reliably corrupts QuickJS's GC state (a real `JS_FreeRuntime` assertion failure hit during
//! development — reading/constructing values natively from the same spot is fine, only invoking a
//! JS-level function call from there isn't).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rquickjs::prelude::{Async, Rest};
use rquickjs::{
    Array, AsyncContext, AsyncRuntime, CatchResultExt, Ctx, Function, Object, Promise, Type, Value,
};

/// A named async function the script can call via `await <name>(...)`. Arguments arrive already
/// parsed from JSON (each JS argument converted natively — see the module doc); the return value
/// is converted back the same way. `Err` rejects the calling script's promise with the message as
/// the JS exception text.
pub struct HostFunction {
    pub name: String,
    #[allow(clippy::type_complexity)]
    pub call: Arc<
        dyn Fn(Vec<serde_json::Value>) -> Pin<Box<dyn Future<Output = HostResult> + Send>>
            + Send
            + Sync,
    >,
}

pub type HostResult = Result<serde_json::Value, String>;

impl HostFunction {
    pub fn new<F, Fut>(name: impl Into<String>, call: F) -> Self
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HostResult> + Send + 'static,
    {
        HostFunction {
            name: name.into(),
            call: Arc::new(move |args| Box::pin(call(args))),
        }
    }
}

/// Hard execution limits for one script run. Scripts are model-authored fresh every turn, so
/// runaway shapes are a WHEN, not an IF: a `while (true) {}` before the first `await` would
/// otherwise hang the session unrecoverably (tokio task-abort can only fire at an await point,
/// and a synchronous QuickJS interpretation never reaches one), and unbounded string/array
/// growth would OOM the whole process rather than fail the one script.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// QuickJS heap ceiling; an allocation past it fails and surfaces as a script error.
    pub memory_bytes: usize,
    /// Longest CONTINUOUS synchronous execution slice before the interpreter is interrupted.
    /// Time suspended awaiting a host function does not count — only pure JS compute since the
    /// last host-function return.
    pub max_sync_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_bytes: 256 * 1024 * 1024,
            max_sync_ms: 30_000,
        }
    }
}

/// Wall-clock bookkeeping for the synchronous-slice cap: the interrupt handler (called by
/// QuickJS mid-interpretation) compares "now" against the last time control returned from a
/// host function. Awaiting an `agent()` call for minutes is fine; computing in a tight JS loop
/// for `max_sync_ms` without ever calling a host function is what gets interrupted.
struct ExecClock {
    started: Instant,
    last_host_return_ms: AtomicU64,
}

impl ExecClock {
    fn sync_elapsed_ms(&self) -> u64 {
        (self.started.elapsed().as_millis() as u64)
            .saturating_sub(self.last_host_return_ms.load(Ordering::Relaxed))
    }

    fn mark_host_return(&self) {
        self.last_host_return_ms
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

/// Runs `script` (evaluated as an async-IIFE — a bare top-level `await` isn't valid outside a JS
/// module) with the given host functions registered as globals, and returns whatever value the
/// script's top-level promise resolves to. `Err` surfaces a script exception or a setup failure
/// as a plain message (this is a sandboxed script, not a place to hand back rich Rust errors).
pub async fn run_script(
    host_fns: Vec<HostFunction>,
    script: &str,
) -> Result<serde_json::Value, String> {
    run_script_with_limits(host_fns, script, Limits::default()).await
}

/// [`run_script`] with explicit [`Limits`] — split out so tests (and any future config knob)
/// can use tight limits without waiting out the production defaults.
pub async fn run_script_with_limits(
    host_fns: Vec<HostFunction>,
    script: &str,
    limits: Limits,
) -> Result<serde_json::Value, String> {
    let rt = AsyncRuntime::new().map_err(|e| format!("failed to create JS runtime: {e}"))?;
    rt.set_memory_limit(limits.memory_bytes).await;
    let clock = Arc::new(ExecClock {
        started: Instant::now(),
        last_host_return_ms: AtomicU64::new(0),
    });
    {
        let clock = Arc::clone(&clock);
        let cap = limits.max_sync_ms;
        rt.set_interrupt_handler(Some(Box::new(move || clock.sync_elapsed_ms() > cap)))
            .await;
    }
    // Drives the runtime's internal job queue for as long as `rt` (and this future) lives — this
    // is what lets multiple concurrently-awaited host-function calls (e.g. inside `parallel()`)
    // actually make progress at once, not just one at a time. `DriveFuture` only holds a WEAK
    // reference to the runtime, so it exits on its own once `rt` drops at the end of this
    // function — do NOT `.abort()` it: that can interrupt QuickJS mid-operation and corrupt its
    // internal GC bookkeeping (hit a real `JS_FreeRuntime` assertion failure doing exactly that
    // during development). Fire-and-forget, exactly as proven in PR0's spike.
    tokio::spawn(rt.drive());

    let ctx = AsyncContext::full(&rt)
        .await
        .map_err(|e| format!("failed to create JS context: {e}"))?;

    let script = script.to_string();
    let result = ctx
        .async_with(async |ctx| -> Result<serde_json::Value, String> {
            for host_fn in host_fns {
                register(&ctx, host_fn, Arc::clone(&clock))
                    .map_err(|e| format!("failed to register host function: {e}"))?;
            }

            let iife: Function = ctx
                .eval(script)
                .catch(&ctx)
                .map_err(|e| format!("script parse/setup error: {e}"))?;
            let promise: Promise = iife
                .call(())
                .catch(&ctx)
                .map_err(|e| format!("script setup error: {e}"))?;
            let value: Value = promise
                .into_future()
                .await
                .catch(&ctx)
                .map_err(|e| format!("script error: {e}"))?;
            js_to_json(&value).map_err(|e| format!("failed to read script result: {e}"))
        })
        .await;

    // A QuickJS interrupt surfaces as a bare "interrupted" — spell out what tripped it so the
    // authoring model's retry fixes the loop instead of guessing.
    result.map_err(|e| {
        if e.to_lowercase().contains("interrupt") {
            format!(
                "{e} (the script computed for more than {}s straight without calling a host \
                 function — an accidental infinite loop? — and was stopped)",
                limits.max_sync_ms.max(1000) / 1000
            )
        } else {
            e
        }
    })
}

/// Registers one host function as a JS global. The wrapper: convert each JS argument to JSON
/// natively (no JS-level function call — see the module doc), call the Rust function, convert
/// its JSON result back the same way. Errors from the Rust function become a rejected promise (a
/// thrown JS exception at the `await` call site).
///
/// Critical: `ctx` is received as a genuine call PARAMETER (`Ctx<'js>` implements `FromParam`,
/// so rquickjs supplies a fresh one per call) — NOT captured into the closure's environment via
/// `ctx.clone()` from outside. Capturing an extra `Ctx` clone into more than one registered
/// function's closure reliably corrupts QuickJS's GC bookkeeping (a real `JS_FreeRuntime`
/// assertion failure, reproduced and bisected during development down to exactly this).
fn register<'js>(
    ctx: &Ctx<'js>,
    host_fn: HostFunction,
    clock: Arc<ExecClock>,
) -> rquickjs::Result<()> {
    let call = host_fn.call;
    let wrapped = move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        let call = Arc::clone(&call);
        let clock = Arc::clone(&clock);
        async move {
            let mut json_args = Vec::with_capacity(args.0.len());
            for arg in args.0 {
                let json =
                    js_to_json(&arg).map_err(|e| rquickjs::Exception::throw_message(&ctx, &e))?;
                json_args.push(json);
            }
            let result = call(json_args).await;
            // Restart the synchronous-slice clock: whatever JS runs next counts from here.
            clock.mark_host_return();
            match result {
                Ok(value) => Ok(JsonValue(value)),
                Err(msg) => Err(rquickjs::Exception::throw_message(&ctx, &msg)),
            }
        }
    };
    let f = Function::new(ctx.clone(), Async(wrapped))?.with_name(&host_fn.name)?;
    ctx.globals().set(host_fn.name.as_str(), f)?;
    Ok(())
}

/// Wraps a `serde_json::Value` so it can be returned from an async host function — `IntoJs`
/// converts it into a real JS value using only native construction (see [`json_to_js`]).
struct JsonValue(serde_json::Value);

impl<'js> rquickjs::IntoJs<'js> for JsonValue {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        json_to_js(ctx, &self.0)
    }
}

/// Deepest value nesting either conversion will walk. A cyclic JS object (`a.self = a`) makes
/// the tree walk infinite — without this cap that's a Rust stack overflow, which ABORTS the
/// whole process (no catchable panic). 64 is far beyond any legitimate agent-result shape.
const MAX_VALUE_DEPTH: usize = 64;

/// Native recursive conversion, `rquickjs::Value` -> `serde_json::Value`: no JS-level function
/// invocation (see the module doc for why that matters), just type dispatch + native
/// `Object`/`Array`/`String` accessors. Errors are plain strings — both call sites turn them
/// into a JS exception / script-error message anyway.
fn js_to_json<'js>(value: &Value<'js>) -> Result<serde_json::Value, String> {
    js_to_json_at(value, 0)
}

fn js_to_json_at<'js>(value: &Value<'js>, depth: usize) -> Result<serde_json::Value, String> {
    if depth > MAX_VALUE_DEPTH {
        return Err(format!(
            "value nesting exceeds {MAX_VALUE_DEPTH} levels — cyclic objects (or absurdly deep \
             ones) cannot cross the script/host boundary"
        ));
    }
    let err = |e: rquickjs::Error| e.to_string();
    Ok(match value.type_of() {
        Type::Uninitialized | Type::Undefined | Type::Null => serde_json::Value::Null,
        Type::Bool => serde_json::Value::Bool(value.as_bool().unwrap_or(false)),
        Type::Int => serde_json::Value::Number(value.as_int().unwrap_or(0).into()),
        Type::Float => serde_json::Number::from_f64(value.as_float().unwrap_or(0.0))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Type::String => {
            let s = value
                .clone()
                .into_string()
                .ok_or_else(|| "expected String value".to_string())?
                .to_string()
                .map_err(err)?;
            serde_json::Value::String(s)
        }
        Type::Array => {
            let arr = value
                .clone()
                .into_array()
                .ok_or_else(|| "expected Array value".to_string())?;
            let mut out = Vec::with_capacity(arr.len());
            for item in arr.iter::<Value>() {
                out.push(js_to_json_at(&item.map_err(err)?, depth + 1)?);
            }
            serde_json::Value::Array(out)
        }
        // Function/Symbol/etc. have no meaningful JSON representation — same as
        // `JSON.stringify` treating an unrepresentable value as `undefined`.
        Type::Object => {
            let obj = value
                .clone()
                .into_object()
                .ok_or_else(|| "expected Object value".to_string())?;
            let mut map = serde_json::Map::new();
            for key in obj.keys::<String>() {
                let key = key.map_err(err)?;
                let v: Value = obj.get(&key).map_err(err)?;
                map.insert(key, js_to_json_at(&v, depth + 1)?);
            }
            serde_json::Value::Object(map)
        }
        _ => serde_json::Value::Null,
    })
}

/// Native recursive conversion, `serde_json::Value` -> `rquickjs::Value` — the reverse of
/// [`js_to_json`], same "no JS-level function invocation" rule and the same depth cap
/// (`serde_json::Value` can't be cyclic, but host functions shouldn't be able to hand a script
/// something the return trip would then refuse).
fn json_to_js<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> rquickjs::Result<Value<'js>> {
    json_to_js_at(ctx, value, 0)
}

fn json_to_js_at<'js>(
    ctx: &Ctx<'js>,
    value: &serde_json::Value,
    depth: usize,
) -> rquickjs::Result<Value<'js>> {
    if depth > MAX_VALUE_DEPTH {
        return Err(rquickjs::Exception::throw_message(
            ctx,
            &format!("value nesting exceeds {MAX_VALUE_DEPTH} levels"),
        ));
    }
    Ok(match value {
        serde_json::Value::Null => Value::new_null(ctx.clone()),
        serde_json::Value::Bool(b) => Value::new_bool(ctx.clone(), *b),
        serde_json::Value::Number(n) => Value::new_float(ctx.clone(), n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => rquickjs::String::from_str(ctx.clone(), s)?.into_value(),
        serde_json::Value::Array(items) => {
            let arr = Array::new(ctx.clone())?;
            for (i, item) in items.iter().enumerate() {
                arr.set(i, json_to_js_at(ctx, item, depth + 1)?)?;
            }
            arr.into_value()
        }
        serde_json::Value::Object(map) => {
            let obj = Object::new(ctx.clone())?;
            for (k, v) in map {
                obj.set(k.as_str(), json_to_js_at(ctx, v, depth + 1)?)?;
            }
            obj.into_value()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A stand-in for a real subagent turn (10-60s of real LLM latency in the eventual feature)
    /// — sleeps, then returns JSON, proving the future genuinely suspends the JS `await` rather
    /// than blocking synchronously.
    fn agent_host_fn() -> HostFunction {
        HostFunction::new("agent", |args| async move {
            let label = args
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(serde_json::Value::String(format!("agent done: {label}")))
        })
    }

    #[tokio::test]
    async fn await_agent_resolves_after_the_real_sleep_completes() {
        let out = run_script(
            vec![agent_host_fn()],
            r#"
            (async () => {
                const result = await agent("hello");
                return result + "!";
            })
            "#,
        )
        .await
        .unwrap();

        assert_eq!(out, "agent done: hello!");
    }

    #[tokio::test]
    async fn concurrent_agent_calls_via_promise_all_run_in_parallel_not_serially() {
        let start = Instant::now();
        let out = run_script(
            vec![agent_host_fn()],
            r#"
            (async () => {
                const [a, b] = await Promise.all([agent("a"), agent("b")]);
                return a + " / " + b;
            })
            "#,
        )
        .await
        .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(out, "agent done: a / agent done: b");
        // A slow/shared CI runner (observed 53ms on macOS CI against a 30ms sleep with a 45ms
        // bound — a real environment-overhead margin issue, not a functional bug) needs a wider
        // gap than a local dev box: sleep long enough, and bound generously enough, that
        // scheduling overhead can't accidentally cross the serialized-vs-concurrent line.
        // Serialized would take 100ms+ (2×50ms); concurrent should land close to 50-70ms even
        // under heavy CI load.
        assert!(
            elapsed < Duration::from_millis(90),
            "expected concurrent execution (~50-70ms), took {elapsed:?} — looks serialized"
        );
    }

    #[tokio::test]
    async fn structured_json_args_and_results_round_trip() {
        let echo = HostFunction::new("echo", |args| async move {
            Ok(args.into_iter().next().unwrap_or(serde_json::Value::Null))
        });
        let out = run_script(
            vec![echo],
            r#"(async () => { return await echo({a: 1, b: [true, "x"]}); })"#,
        )
        .await
        .unwrap();
        assert_eq!(out, serde_json::json!({"a": 1, "b": [true, "x"]}));
    }

    #[tokio::test]
    async fn a_rejected_host_call_surfaces_as_a_script_error() {
        let fails = HostFunction::new("fails", |_args| async move { Err("boom".to_string()) });
        let err = run_script(vec![fails], r#"(async () => { return await fails(); })"#)
            .await
            .unwrap_err();
        assert!(err.contains("boom"), "error message preserved: {err}");
    }

    /// Regression guard for a process-abort class: a cyclic value walked by the recursive
    /// converter used to blow the Rust stack (not a catchable panic — a hard abort of the whole
    /// session). Returning it and passing it to a host function both cross the same boundary.
    #[tokio::test]
    async fn a_cyclic_return_value_errors_instead_of_crashing_the_process() {
        let err = run_script(
            vec![],
            r#"(async () => { const a = {}; a.self = a; return a; })"#,
        )
        .await
        .unwrap_err();
        assert!(err.contains("nesting exceeds"), "explains the cap: {err}");
    }

    #[tokio::test]
    async fn a_cyclic_host_argument_rejects_cleanly() {
        let echo = HostFunction::new("echo", |args| async move {
            Ok(args.into_iter().next().unwrap_or(serde_json::Value::Null))
        });
        let err = run_script(
            vec![echo],
            r#"(async () => { const a = {}; a.self = a; return await echo(a); })"#,
        )
        .await
        .unwrap_err();
        assert!(err.contains("nesting exceeds"), "explains the cap: {err}");
    }

    #[tokio::test]
    async fn legitimately_nested_values_still_round_trip_under_the_depth_cap() {
        let echo = HostFunction::new("echo", |args| async move {
            Ok(args.into_iter().next().unwrap_or(serde_json::Value::Null))
        });
        let out = run_script(
            vec![echo],
            r#"(async () => {
                let v = { leaf: true };
                for (let i = 0; i < 20; i++) v = { child: v };
                const echoed = await echo(v);
                let probe = echoed;
                for (let i = 0; i < 20; i++) probe = probe.child;
                return probe.leaf;
            })"#,
        )
        .await
        .unwrap();
        assert_eq!(out, serde_json::Value::Bool(true));
    }

    /// A synchronous `while(true){}` never reaches an await point, so tokio task-abort can't
    /// stop it — only the QuickJS interrupt handler can. Without it this test would hang the
    /// suite forever.
    #[tokio::test]
    async fn a_runaway_synchronous_loop_is_interrupted_not_hung() {
        let start = Instant::now();
        let err = run_script_with_limits(
            vec![],
            r#"(async () => { while (true) {} })"#,
            Limits {
                max_sync_ms: 200,
                ..Limits::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            err.to_lowercase().contains("interrupt"),
            "interrupt surfaced: {err}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "stopped promptly, took {:?}",
            start.elapsed()
        );
    }

    /// The sync-slice clock restarts at every host-function return — long AWAITS must not trip
    /// the cap, only long uninterrupted compute.
    #[tokio::test]
    async fn a_slow_host_call_does_not_trip_the_sync_slice_cap() {
        let slow = HostFunction::new("slow", |_args| async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(serde_json::Value::String("done".into()))
        });
        let out = run_script_with_limits(
            vec![slow],
            r#"(async () => { return await slow(); })"#,
            Limits {
                max_sync_ms: 200,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(out, "done");
    }

    #[tokio::test]
    async fn unbounded_memory_growth_fails_the_script_not_the_process() {
        let err = run_script_with_limits(
            vec![],
            r#"(async () => { let s = "x".repeat(1024); while (true) { s += s; } })"#,
            Limits {
                memory_bytes: 16 * 1024 * 1024,
                ..Limits::default()
            },
        )
        .await
        .unwrap_err();
        // Either the allocator refuses (out of memory) or the interrupt fires first on a slow
        // machine — both are acceptable "stopped cleanly" outcomes.
        assert!(!err.is_empty());
    }

    /// The whole sandboxing guarantee: a script only ever gets the functions the caller
    /// registered — there is no ambient `require`/`fs`/`fetch`/etc. to escape with.
    #[tokio::test]
    async fn unregistered_globals_are_not_available() {
        let err = run_script(
            vec![],
            r#"(async () => { return typeof require !== "undefined" ? require("fs") : "no fs"; })"#,
        )
        .await
        .unwrap();
        assert_eq!(err, "no fs");
    }
}
