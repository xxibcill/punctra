use std::fmt::{self, Write as _};

pub(crate) const MAX_DIAGNOSTIC_BYTES: usize = 1_024;
const ELLIPSIS: &str = "...";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedDiagnostic(Box<str>);

impl BoundedDiagnostic {
    pub(crate) fn new(message: impl fmt::Display) -> Self {
        let mut output = CappedFormatter::new();
        let _ = write!(&mut output, "{message}");
        Self(output.text.into_boxed_str())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BoundedDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

struct CappedFormatter {
    text: String,
    truncated: bool,
}

impl CappedFormatter {
    fn new() -> Self {
        let mut text = String::new();
        let _ = text.try_reserve_exact(MAX_DIAGNOSTIC_BYTES);
        Self {
            text,
            truncated: false,
        }
    }
}

impl fmt::Write for CappedFormatter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let remaining = MAX_DIAGNOSTIC_BYTES.saturating_sub(self.text.len());
        if value.len() <= remaining {
            self.text.push_str(value);
            return Ok(());
        }
        let mut end = remaining;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        self.text.push_str(&value[..end]);
        let target = MAX_DIAGNOSTIC_BYTES - ELLIPSIS.len();
        while self.text.len() > target {
            self.text.pop();
        }
        self.text.push_str(ELLIPSIS);
        self.truncated = true;
        Ok(())
    }
}
