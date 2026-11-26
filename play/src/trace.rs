//! `--trace <file>.json` (`WORLD_PLAN.md`, phase E): the engine's spans (an action applied, its rule
//! and tool checks, promotion, water, commits) written as Chrome trace events, to be read in the
//! Perfetto UI (ui.perfetto.dev) or `chrome://tracing`. Timing only: nothing the world does depends
//! on it.
//!
//! A layer of Bevy's own tracing subscriber (`LogPlugin::custom_layer`), keeping only the spans of
//! Plateau's crates, so the file stays small.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::span::{Attributes, Id};
use bevy::log::tracing::Subscriber;
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::registry::LookupSpan;
use bevy::log::BoxedLayer;
use bevy::prelude::App;

/// Where the trace goes, if anywhere: set before the app is built.
pub static TRACE_TO: OnceLock<PathBuf> = OnceLock::new();

/// `LogPlugin::custom_layer`: a Chrome trace layer if `--trace` was given.
pub fn layer(_app: &mut App) -> Option<BoxedLayer> {
    let path = TRACE_TO.get()?;
    match ChromeTrace::create(path) {
        Ok(layer) => {
            eprintln!("tracing to {} (open it in ui.perfetto.dev)", path.display());
            Some(Box::new(layer))
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            None
        }
    }
}

struct ChromeTrace {
    out: Mutex<BufWriter<File>>,
    start: Instant,
}

/// What a span keeps until it closes: its name, its fields as JSON, and when it opened.
struct Open {
    name: &'static str,
    args: String,
    at: Instant,
}

/// A span's fields, as the members of a JSON object.
#[derive(Default)]
struct Args(String);

impl Visit for Args {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(',');
        }
        let text = format!("{value:?}");
        let text = text.trim_matches('"');
        self.0.push_str(&format!("{}:{}", json_string(field.name()), json_string(text)));
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl ChromeTrace {
    fn create(path: &PathBuf) -> std::io::Result<ChromeTrace> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::new(File::create(path)?);
        // An array left open is what Chrome's trace format allows for a trace cut off at any time.
        out.write_all(b"[\n")?;
        out.flush()?;
        Ok(ChromeTrace { out: Mutex::new(out), start: Instant::now() })
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for ChromeTrace {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let meta = attrs.metadata();
        if !meta.target().starts_with("plateau") {
            return;
        }
        let mut args = Args::default();
        attrs.record(&mut args);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(Open { name: meta.name(), args: args.0, at: Instant::now() });
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(open) = span.extensions_mut().remove::<Open>() else { return };
        let ts = open.at.duration_since(self.start).as_secs_f64() * 1e6;
        let dur = open.at.elapsed().as_secs_f64() * 1e6;
        let tid = format!("{:?}", std::thread::current().id());
        let tid: u64 = tid.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
        let line = format!(
            "{{\"name\":{},\"cat\":\"plateau\",\"ph\":\"X\",\"ts\":{ts:.3},\"dur\":{dur:.3},\"pid\":1,\"tid\":{tid},\"args\":{{{}}}}},\n",
            json_string(open.name),
            open.args
        );
        if let Ok(mut out) = self.out.lock() {
            let _ = out.write_all(line.as_bytes());
            let _ = out.flush();
        }
    }
}
