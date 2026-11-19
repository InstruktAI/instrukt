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

/// `error` as the one line a failing command ends with: its causes after it,
/// each only when the message before it does not already hold it, and a
/// message of several lines joined with `; ` (after a line ending in `:`,
/// with a space).
pub fn error_line(error: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in error.chain() {
        let text = cause.to_string();
        if parts
            .last()
            .is_some_and(|before| before.contains(text.trim()))
        {
            continue;
        }
        parts.push(text);
    }
    let mut line = String::new();
    for piece in parts
        .join(": ")
        .lines()
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
    {
        if !line.is_empty() {
            line.push_str(if line.ends_with(':') { " " } else { "; " });
        }
        line.push_str(piece);
    }
    line
}

/// Ends a command that meant to act and could not, after its output: its
/// one stderr line, recorded in the log, and the failure status. A command
/// whose output is a report of findings ends without it.
pub fn could_not(line: &str) -> std::process::ExitCode {
    tracing::error!(event = "command_failed", error = %line);
    eprintln!("instrukt: {line}");
    std::process::ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::error_line;

    #[test]
    fn an_error_is_one_line_holding_each_cause_once() {
        let error = anyhow::anyhow!("did not find expected ']' at line 2 column 1")
            .context(
                "/m/instrukt.yaml is not valid YAML: did not find expected ']' at line 2 column 1",
            )
            .context("cannot read adopted-project demand");
        insta::assert_snapshot!(error_line(&error), @"cannot read adopted-project demand: /m/instrukt.yaml is not valid YAML: did not find expected ']' at line 2 column 1");
        let refused =
            anyhow::anyhow!("refused; nothing was written:\na: unknown key\nb: not a list\n");
        insta::assert_snapshot!(error_line(&refused), @"refused; nothing was written: a: unknown key; b: not a list");
    }
}
