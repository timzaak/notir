use anyhow::{Context as AnyhowContext, Result};
use rquickjs::{Context, Ctx, Function, Runtime};
use std::fs;

use crate::script_api::PASSTHROUGH_SCRIPT;

/// JS transform engine backed by rquickjs.
///
/// **Not `Send`** — rquickjs Runtime is not thread-safe. Must be used from a single async task only.
/// Do not move across tokio task boundaries.
pub struct JsEngine {
    ctx: Context,
}

impl JsEngine {
    pub fn new(script_path: Option<&str>) -> Result<Self> {
        let script = match script_path {
            Some(path) => fs::read_to_string(path)
                .with_context(|| format!("Failed to read script file: {}", path))?,
            None => PASSTHROUGH_SCRIPT.to_string(),
        };

        let runtime = Runtime::new().with_context(|| "Failed to create JS runtime")?;
        let ctx = Context::full(&runtime).with_context(|| "Failed to create JS context")?;

        ctx.with(|ctx| {
            inject_console(&ctx)?;
            ctx.eval::<(), _>(script.as_str())
                .map_err(|e| anyhow::anyhow!("JS script error: {:?}", e))?;
            Ok::<(), anyhow::Error>(())
        })
        .with_context(|| "Failed to initialize JS context")?;

        Ok(Self { ctx })
    }

    pub fn transform(&self, event_json: &str) -> Result<Option<String>> {
        self.ctx.with(|ctx: Ctx| {
            let func: Function = ctx
                .eval("transform")
                .map_err(|e| anyhow::anyhow!("JS: transform not found: {:?}", e))?;

            let json_val = ctx
                .json_parse(event_json)
                .map_err(|e| anyhow::anyhow!("JS JSON parse error: {:?}", e))?;

            let result: Option<String> = func
                .call((json_val,))
                .map_err(|e| anyhow::anyhow!("JS transform error: {:?}", e))?;
            Ok(result)
        })
    }
}

fn inject_console(ctx: &Ctx) -> Result<()> {
    let global = ctx.globals();
    let console = rquickjs::Object::new(ctx.clone())?;
    console.set(
        "log",
        Function::new(ctx.clone(), |args: Vec<String>| {
            eprintln!("{}", args.join(" "));
        })?,
    )?;
    console.set(
        "error",
        Function::new(ctx.clone(), |args: Vec<String>| {
            eprintln!("[ERROR] {}", args.join(" "));
        })?,
    )?;
    global.set("console", console)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passthrough_text() {
        let engine = JsEngine::new(None).unwrap();
        let event = r#"{"text":"hello","binary":null,"timestamp":"2026-01-01T00:00:00Z","type":"text","source":"single"}"#;
        let result = engine.transform(event).unwrap();
        assert_eq!(result, Some("hello".to_string()));
    }

    #[test]
    fn test_passthrough_binary() {
        let engine = JsEngine::new(None).unwrap();
        let event = r#"{"text":null,"binary":"deadbeef","timestamp":"2026-01-01T00:00:00Z","type":"binary","source":"single"}"#;
        let result = engine.transform(event).unwrap();
        assert_eq!(result, Some("deadbeef".to_string()));
    }

    #[test]
    fn test_custom_transform() {
        let script = r#"
            function transform(e) {
                var data = JSON.parse(e.text);
                return JSON.stringify({ receivedAt: e.timestamp, payload: data });
            }
        "#;
        let dir = std::env::temp_dir().join("notir_test_transform.js");
        std::fs::write(&dir, script).unwrap();
        let engine = JsEngine::new(Some(dir.to_str().unwrap())).unwrap();
        let event = r#"{"text":"{\"msg\":\"hi\"}","binary":null,"timestamp":"2026-01-01T00:00:00Z","type":"text","source":"single"}"#;
        let result = engine.transform(event).unwrap();
        assert!(result.is_some());
        let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(parsed["receivedAt"], "2026-01-01T00:00:00Z");
        assert_eq!(parsed["payload"]["msg"], "hi");
    }

    #[test]
    fn test_filter_returns_null() {
        let script = r#"
            function transform(e) {
                return null;
            }
        "#;
        let dir = std::env::temp_dir().join("notir_test_filter.js");
        std::fs::write(&dir, script).unwrap();
        let engine = JsEngine::new(Some(dir.to_str().unwrap())).unwrap();
        let event =
            r#"{"text":"hello","binary":null,"timestamp":"","type":"text","source":"single"}"#;
        let result = engine.transform(event).unwrap();
        assert_eq!(result, None);
    }
}
