//! Shared depth / result mode parsing (tool_spec v2).

use crate::envelope::{ErrorCode, Fail};

/// Host payload shape for live tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultMode {
    Digest,
    Evidence,
    Both,
}

impl ResultMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Digest => "digest",
            Self::Evidence => "evidence",
            Self::Both => "both",
        }
    }

    #[must_use]
    pub fn wants_digest(self) -> bool {
        matches!(self, Self::Digest | Self::Both)
    }

    #[must_use]
    pub fn wants_evidence(self) -> bool {
        matches!(self, Self::Evidence | Self::Both)
    }
}

/// Parse `result` query param. Default `digest`.
pub fn parse_result_mode(raw: Option<&str>) -> Result<ResultMode, Fail> {
    let s = raw.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("digest");
    match s.to_ascii_lowercase().as_str() {
        "digest" => Ok(ResultMode::Digest),
        "evidence" => Ok(ResultMode::Evidence),
        "both" => Ok(ResultMode::Both),
        other => Err(Fail::new(
            ErrorCode::InvalidParams,
            format!("result must be digest|evidence|both, got {other}"),
            false,
        )),
    }
}

/// Maps `depth` → Responses reasoning effort. Default `standard` → medium.
pub fn parse_depth_effort(raw: Option<&str>) -> Result<&'static str, Fail> {
    let s = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("standard");
    match s.to_ascii_lowercase().as_str() {
        "quick" => Ok("low"),
        "standard" => Ok("medium"),
        "deep" => Ok("high"),
        // accept legacy effort names if a host still sends them
        "low" => Ok("low"),
        "medium" => Ok("medium"),
        "high" => Ok("high"),
        other => Err(Fail::new(
            ErrorCode::InvalidParams,
            format!("depth must be quick|standard|deep, got {other}"),
            false,
        )),
    }
}

/// Host-facing character budget for primary text fields.
#[must_use]
pub fn result_char_budget(mode: ResultMode) -> usize {
    match mode {
        ResultMode::Digest => 6 * 1024,
        ResultMode::Evidence | ResultMode::Both => 48 * 1024,
    }
}

/// Per-post text cap.
#[must_use]
pub fn post_text_cap(mode: ResultMode) -> usize {
    match mode {
        ResultMode::Digest => 400,
        ResultMode::Evidence | ResultMode::Both => 4000,
    }
}

/// ask_grok / offline budgets from depth effort string.
#[must_use]
pub fn depth_char_budget(effort: &str) -> usize {
    match effort {
        "low" => 4 * 1024,
        "high" => 16 * 1024,
        _ => 8 * 1024,
    }
}

#[must_use]
pub fn cost_hint_for(tool: &str, effort: &str, mode: Option<ResultMode>) -> &'static str {
    if tool == "ask_grok" {
        return match effort {
            "high" => "mid",
            _ => "low",
        };
    }
    if tool == "research" {
        return match effort {
            "low" => "mid",
            _ => "high",
        };
    }
    // x_search
    match (effort, mode.unwrap_or(ResultMode::Digest)) {
        ("high", _) | (_, ResultMode::Evidence | ResultMode::Both) => "high",
        ("low", ResultMode::Digest) => "mid",
        _ => "mid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_defaults_and_parses() {
        assert_eq!(parse_result_mode(None).unwrap(), ResultMode::Digest);
        assert_eq!(parse_result_mode(Some("EVIDENCE")).unwrap(), ResultMode::Evidence);
        assert!(parse_result_mode(Some("raw")).is_err());
    }

    #[test]
    fn depth_maps_to_effort() {
        assert_eq!(parse_depth_effort(None).unwrap(), "medium");
        assert_eq!(parse_depth_effort(Some("quick")).unwrap(), "low");
        assert_eq!(parse_depth_effort(Some("deep")).unwrap(), "high");
    }

    #[test]
    fn evidence_wants() {
        assert!(ResultMode::Both.wants_digest());
        assert!(ResultMode::Both.wants_evidence());
        assert!(!ResultMode::Digest.wants_evidence());
    }
}
