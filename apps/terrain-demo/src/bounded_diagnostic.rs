use std::fmt;

pub(crate) const MAX_DIAGNOSTIC_BYTES: usize = 1_024;
const ELLIPSIS: &str = "...";
const ALLOCATION_FAILURE: &str = "diagnostic unavailable because its bounded allocation failed";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundedDiagnostic {
    text: DiagnosticText,
    truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DiagnosticText {
    Owned(String),
    Static(&'static str),
}

impl BoundedDiagnostic {
    pub(crate) fn new(message: impl fmt::Display) -> Self {
        Self::new_with_storage(message, reserved_storage())
    }

    fn new_with_storage(message: impl fmt::Display, storage: Option<String>) -> Self {
        let Some(storage) = storage else {
            return Self {
                text: DiagnosticText::Static(ALLOCATION_FAILURE),
                truncated: false,
            };
        };
        let mut output = CappedFormatter::new(storage);
        let _ = fmt::write(&mut output, format_args!("{message}"));
        output.finish()
    }

    pub(crate) fn as_str(&self) -> &str {
        match &self.text {
            DiagnosticText::Owned(text) => text,
            DiagnosticText::Static(text) => text,
        }
    }

    #[cfg(test)]
    const fn was_truncated(&self) -> bool {
        self.truncated
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
    fn new(text: String) -> Self {
        debug_assert!(text.capacity() >= MAX_DIAGNOSTIC_BYTES);
        Self {
            text,
            truncated: false,
        }
    }

    fn finish(self) -> BoundedDiagnostic {
        BoundedDiagnostic {
            text: DiagnosticText::Owned(self.text),
            truncated: self.truncated,
        }
    }

    fn truncate(&mut self) {
        let target = MAX_DIAGNOSTIC_BYTES - ELLIPSIS.len();
        while self.text.len() > target {
            self.text.pop();
        }
        self.text.push_str(ELLIPSIS);
        self.truncated = true;
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
        self.truncate();
        Ok(())
    }
}

fn reserved_storage() -> Option<String> {
    let mut text = String::new();
    text.try_reserve_exact(MAX_DIAGNOSTIC_BYTES).ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_boundary_is_preserved_without_truncation() {
        let diagnostic = BoundedDiagnostic::new("x".repeat(MAX_DIAGNOSTIC_BYTES));

        assert_eq!(diagnostic.as_str().len(), MAX_DIAGNOSTIC_BYTES);
        assert!(!diagnostic.was_truncated());
        assert!(diagnostic.as_str().bytes().all(|byte| byte == b'x'));
    }

    #[test]
    fn over_boundary_is_utf8_safe_and_ends_with_an_ellipsis() {
        let diagnostic = BoundedDiagnostic::new("ก".repeat(MAX_DIAGNOSTIC_BYTES));

        assert!(diagnostic.as_str().len() <= MAX_DIAGNOSTIC_BYTES);
        assert!(diagnostic.as_str().len() >= MAX_DIAGNOSTIC_BYTES - 3);
        assert!(diagnostic.was_truncated());
        assert!(diagnostic.as_str().ends_with(ELLIPSIS));
    }

    #[test]
    fn construction_uses_one_fallible_allocation_without_shrinking() {
        let external = "x".repeat(MAX_DIAGNOSTIC_BYTES);
        let mut diagnostic = None;
        let allocations = allocation_counter::measure(|| {
            diagnostic = Some(BoundedDiagnostic::new(format_args!(
                "bounded external diagnostic: {external}"
            )));
        });
        let diagnostic = diagnostic.expect("measurement records a diagnostic");

        assert!(diagnostic.was_truncated());
        assert_eq!(allocations.count_current, 1);
        assert_eq!(allocations.count_total, 1);
        assert_eq!(
            allocations.bytes_current,
            i64::try_from(MAX_DIAGNOSTIC_BYTES).expect("diagnostic limit fits i64")
        );
    }

    #[test]
    fn allocation_failure_has_a_static_zero_allocation_fallback() {
        let allocations = allocation_counter::measure(|| {
            let diagnostic = BoundedDiagnostic::new_with_storage("external detail", None);
            assert_eq!(diagnostic.as_str(), ALLOCATION_FAILURE);
            assert!(!diagnostic.was_truncated());
        });

        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
}
