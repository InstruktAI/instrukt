//! The command output writer: the one place stdout is written.
//!
//! Commands write their contract output (text agents read, or JSON) here and
//! never print directly; diagnostics go to the command's log through `tracing`.

use std::io::{self, Write};

use anyhow::Context as _;
use serde::Serialize;

pub struct Output {
    sink: Box<dyn Write>,
}

impl Output {
    pub fn stdout() -> Self {
        Self {
            sink: Box::new(io::BufWriter::new(io::stdout())),
        }
    }

    /// Writes `text` as is.
    pub fn text(&mut self, text: &str) -> anyhow::Result<()> {
        self.write(text.as_bytes())
    }

    /// Writes `text` followed by a newline.
    pub fn line(&mut self, text: &str) -> anyhow::Result<()> {
        self.write(text.as_bytes())?;
        self.write(b"\n")
    }

    /// Writes `value` as pretty JSON followed by a newline.
    pub fn json<T: Serialize + ?Sized>(&mut self, value: &T) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(value).context("cannot render JSON output")?;
        self.line(&text)
    }

    pub fn flush(&mut self) -> anyhow::Result<()> {
        match self.sink.flush() {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            other => other.context("cannot write to stdout"),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        match self.sink.write_all(bytes) {
            // A reader that stopped reading (`| head`) is not an error.
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            other => other.context("cannot write to stdout"),
        }
    }
}
