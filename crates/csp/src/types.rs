//! Core domain types. Port of `src/types.ts` (← semble `types.py`).
//!
//! The dict helpers are the on-disk / round-trip representation of a [`Chunk`]:
//! camelCase field names plus a derived `location`. `chunk_from_dict` validates
//! untrusted JSON (the Rust counterpart of the TypeScript `TypeError` guards) so
//! corrupt input cannot pollute the index.

use serde::{Deserialize, Serialize};

/// Call type for token-savings tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallType {
    #[serde(rename = "search")]
    Search,
    // Python uses `find_related` (snake_case) — telemetry compatibility.
    #[serde(rename = "find_related")]
    FindRelated,
}

/// Content type for indexing and search pipeline selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Code,
    Docs,
    Config,
}

/// A single indexable unit of code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub content: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub language: Option<String>,
}

/// A chunk serialized to a plain camelCase dict (e.g. for `chunks.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkDict {
    pub content: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    /// `null` when absent (matching Python `asdict`'s `None`).
    pub language: Option<String>,
    pub location: String,
}

/// A search result serialized to a camelCase dict, embedding [`ChunkDict`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultDict {
    pub chunk: ChunkDict,
    pub score: f64,
}

/// Error raised when reconstructing a [`Chunk`] from untrusted JSON.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("chunkFromDict: {0}")]
pub struct ChunkFromDictError(&'static str);

/// Format a chunk's source location as `filePath:startLine-endLine`.
pub fn chunk_location(chunk: &Chunk) -> String {
    format!(
        "{}:{}-{}",
        chunk.file_path, chunk.start_line, chunk.end_line
    )
}

/// Serialize a [`Chunk`] to a camelCase [`ChunkDict`], appending a derived
/// `location`. `language` is normalized to `null` when absent.
pub fn chunk_to_dict(chunk: &Chunk) -> ChunkDict {
    ChunkDict {
        content: chunk.content.clone(),
        file_path: chunk.file_path.clone(),
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        language: chunk.language.clone(),
        location: chunk_location(chunk),
    }
}

/// A finite, non-negative integer line number, or `None` for any other JSON
/// value. Mirrors the TypeScript `isFiniteNumber` guard; JSON cannot represent
/// `NaN`/`Infinity`, so those JS-only cases are unrepresentable here by design.
fn as_line_number(value: Option<&serde_json::Value>) -> Option<u32> {
    value
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// Reconstruct a [`Chunk`] from an untrusted JSON value. The derived `location`
/// is ignored (never trusted — recomputed from the line range), a `null`/absent
/// language collapses to `None`, and malformed input is rejected.
pub fn chunk_from_dict(value: &serde_json::Value) -> Result<Chunk, ChunkFromDictError> {
    let obj = value
        .as_object()
        .ok_or(ChunkFromDictError("expected an object"))?;

    let content = obj
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or(ChunkFromDictError("`content` must be a string"))?;
    let file_path = obj
        .get("filePath")
        .and_then(serde_json::Value::as_str)
        .ok_or(ChunkFromDictError("`filePath` must be a string"))?;
    let start_line = as_line_number(obj.get("startLine"))
        .ok_or(ChunkFromDictError("`startLine` must be a finite number"))?;
    let end_line = as_line_number(obj.get("endLine"))
        .ok_or(ChunkFromDictError("`endLine` must be a finite number"))?;
    let language = match obj.get("language") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return Err(ChunkFromDictError(
                "`language` must be a string, null, or omitted",
            ))
        }
    };

    Ok(Chunk {
        content: content.to_string(),
        file_path: file_path.to_string(),
        start_line,
        end_line,
        language,
    })
}

/// Serialize a `{ chunk, score }` result to a camelCase [`SearchResultDict`].
pub fn search_result_to_dict(chunk: &Chunk, score: f64) -> SearchResultDict {
    SearchResultDict {
        chunk: chunk_to_dict(chunk),
        score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Mirrors src/types.test.ts (port-parity with semble test_types.py).

    #[test]
    fn content_type_enum_values_match() {
        assert_eq!(
            serde_json::to_value(ContentType::Code).unwrap(),
            json!("code")
        );
        assert_eq!(
            serde_json::to_value(ContentType::Docs).unwrap(),
            json!("docs")
        );
        assert_eq!(
            serde_json::to_value(ContentType::Config).unwrap(),
            json!("config")
        );
    }

    #[test]
    fn call_type_enum_values_match() {
        assert_eq!(
            serde_json::to_value(CallType::Search).unwrap(),
            json!("search")
        );
        assert_eq!(
            serde_json::to_value(CallType::FindRelated).unwrap(),
            json!("find_related")
        );
    }

    #[test]
    fn chunk_location_formats_path_and_range() {
        let chunk = Chunk {
            content: "x = 1".into(),
            file_path: "file.ts".into(),
            start_line: 10,
            end_line: 25,
            language: None,
        };
        assert_eq!(chunk_location(&chunk), "file.ts:10-25");
    }

    #[test]
    fn chunk_location_handles_single_line() {
        let chunk = Chunk {
            content: "x = 1".into(),
            file_path: "src/a.py".into(),
            start_line: 5,
            end_line: 5,
            language: None,
        };
        assert_eq!(chunk_location(&chunk), "src/a.py:5-5");
    }

    #[test]
    fn roundtrip_preserves_fields_with_language() {
        let original = Chunk {
            content: "function foo() {}".into(),
            file_path: "src/foo.ts".into(),
            start_line: 1,
            end_line: 3,
            language: Some("typescript".into()),
        };
        let dict = chunk_to_dict(&original);
        assert_eq!(
            serde_json::to_value(&dict).unwrap(),
            json!({
                "content": "function foo() {}",
                "filePath": "src/foo.ts",
                "startLine": 1,
                "endLine": 3,
                "language": "typescript",
                "location": "src/foo.ts:1-3",
            })
        );
        let reconstructed = chunk_from_dict(&serde_json::to_value(&dict).unwrap()).unwrap();
        assert_eq!(reconstructed, original);
    }

    #[test]
    fn roundtrip_with_language_omitted_emits_null() {
        let original = Chunk {
            content: "README content".into(),
            file_path: "README.md".into(),
            start_line: 1,
            end_line: 10,
            language: None,
        };
        let dict = chunk_to_dict(&original);
        assert_eq!(dict.language, None);
        assert_eq!(dict.location, "README.md:1-10");
        // Serializes to JSON null.
        assert_eq!(
            serde_json::to_value(&dict).unwrap()["language"],
            json!(null)
        );

        let reconstructed = chunk_from_dict(&serde_json::to_value(&dict).unwrap()).unwrap();
        assert_eq!(reconstructed, original);
        assert_eq!(reconstructed.language, None);
    }

    #[test]
    fn from_dict_strips_location_before_reconstruction() {
        let reconstructed = chunk_from_dict(&json!({
            "content": "x",
            "filePath": "a.ts",
            "startLine": 1,
            "endLine": 2,
            "language": "ts",
            "location": "WRONG:999-999",
        }))
        .unwrap();
        assert_eq!(chunk_location(&reconstructed), "a.ts:1-2");
    }

    #[test]
    fn from_dict_accepts_null_language() {
        let reconstructed = chunk_from_dict(&json!({
            "content": "x",
            "filePath": "a.ts",
            "startLine": 1,
            "endLine": 2,
            "language": null,
        }))
        .unwrap();
        assert_eq!(reconstructed.language, None);
    }

    #[test]
    fn from_dict_rejects_non_object() {
        assert!(chunk_from_dict(&json!(null)).is_err());
        assert!(chunk_from_dict(&json!("oops")).is_err());
        assert!(chunk_from_dict(&json!(42)).is_err());
    }

    #[test]
    fn from_dict_rejects_missing_or_wrong_typed_fields() {
        assert!(chunk_from_dict(&json!({})).is_err());
        assert!(
            chunk_from_dict(&json!({ "content": "x", "filePath": "a.ts", "startLine": 1 }))
                .is_err()
        );
        // startLine as a string
        assert!(chunk_from_dict(&json!({
            "content": "x", "filePath": "a.ts", "startLine": "1", "endLine": 2
        }))
        .is_err());
        // filePath as a number
        assert!(chunk_from_dict(&json!({
            "content": "x", "filePath": 42, "startLine": 1, "endLine": 2
        }))
        .is_err());
    }

    #[test]
    fn from_dict_rejects_wrong_typed_language() {
        assert!(chunk_from_dict(&json!({
            "content": "x", "filePath": "a.ts", "startLine": 1, "endLine": 2, "language": 42
        }))
        .is_err());
    }

    #[test]
    fn search_result_to_dict_serialises_chunk_and_score() {
        let chunk = Chunk {
            content: "def foo():\n    pass".into(),
            file_path: "foo.py".into(),
            start_line: 1,
            end_line: 2,
            language: Some("python".into()),
        };
        let dict = search_result_to_dict(&chunk, 0.87);
        assert_eq!(
            serde_json::to_value(&dict).unwrap(),
            json!({
                "chunk": {
                    "content": "def foo():\n    pass",
                    "filePath": "foo.py",
                    "startLine": 1,
                    "endLine": 2,
                    "language": "python",
                    "location": "foo.py:1-2",
                },
                "score": 0.87,
            })
        );
    }
}
