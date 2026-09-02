//! Operation identifiers and the grant patterns that match them.

use std::fmt;

use super::limits::Limits;

/// Why an operation string or pattern was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationError {
    Empty,
    TooLong {
        len: usize,
        max: usize,
    },
    /// A character outside `[a-z0-9_.:-]` (patterns also allow one trailing `*`).
    InvalidCharacter {
        position: usize,
    },
    /// A leading, trailing, or doubled separator.
    MalformedSegment,
    /// A wildcard anywhere but as the final segment.
    MalformedWildcard,
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("operation is empty"),
            Self::TooLong { len, max } => {
                write!(f, "operation has {len} bytes; the limit is {max}")
            }
            Self::InvalidCharacter { position } => {
                write!(f, "operation has an invalid character at byte {position}")
            }
            Self::MalformedSegment => f.write_str("operation has an empty segment"),
            Self::MalformedWildcard => {
                f.write_str("wildcard is only accepted as the final segment, as in `a.b.*`")
            }
        }
    }
}

impl std::error::Error for OperationError {}

/// A normalized operation identifier: lowercase ASCII segments joined by
/// dots, such as `omegon.native_session.read`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Operation(String);

impl Operation {
    /// Parse and normalize an operation. Case is folded; surrounding
    /// whitespace is rejected rather than trimmed so callers notice.
    pub fn parse(raw: &str, limits: &Limits) -> Result<Self, OperationError> {
        let normalized = normalize(raw, limits, false)?;
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a grant matches: one exact operation, or every operation under a
/// suffix-prefix such as `omegon.native_session.*`. Arbitrary globs are not
/// accepted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationPattern {
    Exact(Operation),
    /// Stored with its trailing dot, so `omegon.*` holds `omegon.`.
    Prefix(String),
}

impl OperationPattern {
    pub fn parse(raw: &str, limits: &Limits) -> Result<Self, OperationError> {
        let normalized = normalize(raw, limits, true)?;
        match normalized.strip_suffix(".*") {
            Some(prefix) => Ok(Self::Prefix(format!("{prefix}."))),
            None => Ok(Self::Exact(Operation(normalized))),
        }
    }

    /// Whether the pattern covers `operation`.
    #[must_use]
    pub fn matches(&self, operation: &Operation) -> bool {
        match self {
            Self::Exact(exact) => exact == operation,
            Self::Prefix(prefix) => operation.0.starts_with(prefix.as_str()),
        }
    }

    /// Exact patterns beat prefixes; longer prefixes beat shorter ones.
    #[must_use]
    pub fn specificity(&self) -> usize {
        match self {
            Self::Exact(_) => usize::MAX,
            Self::Prefix(prefix) => prefix.len(),
        }
    }

    #[must_use]
    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }
}

impl fmt::Display for OperationPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(operation) => f.write_str(operation.as_str()),
            Self::Prefix(prefix) => write!(f, "{prefix}*"),
        }
    }
}

fn normalize(raw: &str, limits: &Limits, allow_wildcard: bool) -> Result<String, OperationError> {
    if raw.is_empty() {
        return Err(OperationError::Empty);
    }
    if raw.len() > limits.max_operation_len {
        return Err(OperationError::TooLong { len: raw.len(), max: limits.max_operation_len });
    }
    let lowered = raw.to_ascii_lowercase();
    for (position, byte) in lowered.bytes().enumerate() {
        let ok = byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'.' | b':' | b'-')
            || (allow_wildcard && byte == b'*');
        if !ok {
            return Err(OperationError::InvalidCharacter { position });
        }
    }
    let segments: Vec<&str> = lowered.split('.').collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(OperationError::MalformedSegment);
    }
    let wildcard_count = lowered.matches('*').count();
    if wildcard_count > 1 || (wildcard_count == 1 && segments.last() != Some(&"*")) {
        return Err(OperationError::MalformedWildcard);
    }
    if wildcard_count == 1 && segments.len() < 2 {
        return Err(OperationError::MalformedWildcard);
    }
    Ok(lowered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_normalize_case_and_reject_malformed_input() {
        let limits = Limits::default();
        assert_eq!(
            Operation::parse("Omegon.Surface.Read", &limits).unwrap().as_str(),
            "omegon.surface.read"
        );
        assert_eq!(Operation::parse("", &limits), Err(OperationError::Empty));
        assert_eq!(Operation::parse("a..b", &limits), Err(OperationError::MalformedSegment));
        assert_eq!(
            Operation::parse("a.b.*", &limits),
            Err(OperationError::InvalidCharacter { position: 4 })
        );
        assert!(matches!(
            Operation::parse("a b", &limits),
            Err(OperationError::InvalidCharacter { .. })
        ));
        let long = "a".repeat(limits.max_operation_len + 1);
        assert!(matches!(Operation::parse(&long, &limits), Err(OperationError::TooLong { .. })));
    }

    #[test]
    fn patterns_accept_only_suffix_wildcards() {
        let limits = Limits::default();
        let prefix = OperationPattern::parse("omegon.native_session.*", &limits).unwrap();
        assert_eq!(prefix, OperationPattern::Prefix("omegon.native_session.".into()));
        assert_eq!(prefix.to_string(), "omegon.native_session.*");
        assert!(prefix.matches(&Operation::parse("omegon.native_session.read", &limits).unwrap()));
        assert!(
            !prefix.matches(&Operation::parse("omegon.native_sessions.read", &limits).unwrap())
        );
        assert_eq!(OperationPattern::parse("*", &limits), Err(OperationError::MalformedWildcard));
        assert_eq!(
            OperationPattern::parse("a.*.b", &limits),
            Err(OperationError::MalformedWildcard)
        );
        assert_eq!(
            OperationPattern::parse("a.**", &limits),
            Err(OperationError::MalformedWildcard)
        );
        assert_eq!(OperationPattern::parse("a*", &limits), Err(OperationError::MalformedWildcard));
        let exact = OperationPattern::parse("omegon.surface.read", &limits).unwrap();
        assert!(exact.is_exact());
        assert!(exact.specificity() > prefix.specificity());
    }
}
