//! The fixed shapes a model may answer in, and the check every answer passes
//! before it is believed (research R16: "Schemas stay flat, and responses
//! are validated on the client").
//!
//! A [`Schema`] is declared once, as data, and serves twice: as the JSON
//! Schema the request constrains the runtime's output with
//! (`response_format: json_schema`), and as the check the answer is held
//! to here. A runtime that ignores the constraint -- and some do -- is
//! caught by the second, because nothing about the first is trusted.
//!
//! **Flat.** An answer is one object of plain fields, and at most one field
//! may be a list of rows, each itself an object of plain fields. Small
//! local models keep to a flat shape far more reliably than to a deep one,
//! and a flat shape is one this check can hold completely.

use serde_json::{Map, Value, json};

/// One answer's shape: a name the request carries, and its fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schema {
    /// What the request calls it.
    pub name: &'static str,
    /// Every field, each required.
    pub fields: &'static [Field],
}

/// One field of an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// Its key.
    pub name: &'static str,
    /// What it may hold.
    pub kind: Kind,
}

/// What a field may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A string of at most `max` characters.
    Text {
        /// The most characters it may have.
        max: usize,
    },
    /// One of these strings, exactly.
    Choice(&'static [&'static str]),
    /// A whole number in `min..=max`.
    Integer {
        /// The least it may be.
        min: i64,
        /// The most it may be.
        max: i64,
    },
    /// A day, `YYYY-MM-DD`, or the empty string for none.
    Date,
    /// At most `max` rows, each an object of these fields, none of which
    /// may itself be rows.
    Rows {
        /// The most rows it may hold.
        max: usize,
        /// Each row's fields.
        fields: &'static [Field],
    },
}

/// Why an answer was not believed. Said by field name and rule, never by
/// what the answer held.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// The answer was not a JSON object.
    #[error("the answer is not an object")]
    NotAnObject,
    /// A field the schema requires is missing.
    #[error("the answer has no `{0}`")]
    Missing(&'static str),
    /// A field the schema does not have.
    #[error("the answer has a field the schema does not")]
    Unknown,
    /// A field holds the wrong kind of value, or one out of its bounds.
    #[error("the answer's `{0}` is not what the schema allows")]
    Invalid(&'static str),
}

impl Schema {
    /// The JSON Schema for the request's `response_format`: every field
    /// required, nothing else allowed.
    pub fn json(&self) -> Value {
        object(self.fields)
    }

    /// Whether this schema is flat: no rows inside rows.
    pub fn is_flat(&self) -> bool {
        self.fields.iter().all(|field| match field.kind {
            Kind::Rows { fields, .. } => fields
                .iter()
                .all(|inner| !matches!(inner.kind, Kind::Rows { .. })),
            _ => true,
        })
    }

    /// `answer`, if it keeps to the schema exactly: an object with every
    /// field, no other, each within its kind and bounds.
    pub fn check(&self, answer: Value) -> Result<Map<String, Value>, SchemaError> {
        let Value::Object(fields) = answer else {
            return Err(SchemaError::NotAnObject);
        };
        check_fields(self.fields, &fields)?;
        Ok(fields)
    }
}

fn object(fields: &[Field]) -> Value {
    let properties: Map<String, Value> = fields
        .iter()
        .map(|field| (field.name.to_owned(), property(field.kind)))
        .collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": fields.iter().map(|field| field.name).collect::<Vec<_>>(),
        "additionalProperties": false,
    })
}

fn property(kind: Kind) -> Value {
    match kind {
        Kind::Text { max } => json!({ "type": "string", "maxLength": max }),
        Kind::Choice(choices) => json!({ "type": "string", "enum": choices }),
        Kind::Integer { min, max } => json!({ "type": "integer", "minimum": min, "maximum": max }),
        Kind::Date => json!({ "type": "string", "pattern": "^([0-9]{4}-[0-9]{2}-[0-9]{2})?$" }),
        Kind::Rows { max, fields } => json!({
            "type": "array",
            "maxItems": max,
            "items": object(fields),
        }),
    }
}

fn check_fields(schema: &[Field], fields: &Map<String, Value>) -> Result<(), SchemaError> {
    if fields
        .keys()
        .any(|key| !schema.iter().any(|field| field.name == key))
    {
        return Err(SchemaError::Unknown);
    }
    for field in schema {
        let value = fields
            .get(field.name)
            .ok_or(SchemaError::Missing(field.name))?;
        if !holds(field.kind, value, schema_depth_ok(field.kind))? {
            return Err(SchemaError::Invalid(field.name));
        }
    }
    Ok(())
}

/// Rows may hold only plain fields.
fn schema_depth_ok(kind: Kind) -> bool {
    match kind {
        Kind::Rows { fields, .. } => fields
            .iter()
            .all(|field| !matches!(field.kind, Kind::Rows { .. })),
        _ => true,
    }
}

fn holds(kind: Kind, value: &Value, flat: bool) -> Result<bool, SchemaError> {
    Ok(match (kind, value) {
        (Kind::Text { max }, Value::String(text)) => text.chars().count() <= max,
        (Kind::Choice(choices), Value::String(text)) => choices.contains(&text.as_str()),
        (Kind::Integer { min, max }, Value::Number(number)) => number
            .as_i64()
            .is_some_and(|number| (min..=max).contains(&number)),
        (Kind::Date, Value::String(text)) => {
            text.is_empty() || chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
        }
        (Kind::Rows { max, fields }, Value::Array(rows)) => {
            if !flat || rows.len() > max {
                return Ok(false);
            }
            for row in rows {
                let Value::Object(row) = row else {
                    return Ok(false);
                };
                check_fields(fields, row)?;
            }
            true
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: &[Field] = &[
        Field {
            name: "text",
            kind: Kind::Text { max: 20 },
        },
        Field {
            name: "message",
            kind: Kind::Integer { min: 1, max: 9 },
        },
    ];

    const ANSWER: Schema = Schema {
        name: "answer",
        fields: &[
            Field {
                name: "kind",
                kind: Kind::Choice(&["question", "none"]),
            },
            Field {
                name: "due",
                kind: Kind::Date,
            },
            Field {
                name: "rows",
                kind: Kind::Rows {
                    max: 2,
                    fields: ROW,
                },
            },
        ],
    };

    #[test]
    fn the_request_constrains_the_output_to_the_same_shape() {
        let schema = ANSWER.json();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["kind", "due", "rows"]));
        assert_eq!(
            schema["properties"]["kind"]["enum"],
            json!(["question", "none"])
        );
        assert_eq!(schema["properties"]["rows"]["maxItems"], 2);
        assert_eq!(
            schema["properties"]["rows"]["items"]["required"],
            json!(["text", "message"])
        );
        assert!(ANSWER.is_flat());
    }

    #[test]
    fn an_answer_that_keeps_to_the_schema_is_believed() {
        let answer = json!({
            "kind": "question",
            "due": "2026-09-30",
            "rows": [{ "text": "the vote", "message": 3 }],
        });
        let fields = ANSWER.check(answer).expect("it keeps to the schema");
        assert_eq!(fields["kind"], "question");
        assert!(
            ANSWER
                .check(json!({ "kind": "none", "due": "", "rows": [] }))
                .is_ok()
        );
    }

    #[test]
    fn an_answer_that_strays_from_the_schema_is_not() {
        let good = json!({ "kind": "none", "due": "", "rows": [] });
        let mut cases = Vec::new();
        let mut with = |key: &str, value: Value| {
            let mut answer = good.clone();
            answer[key] = value;
            cases.push(answer);
        };
        with("kind", json!("todo"));
        with("kind", json!(1));
        with("due", json!("Friday"));
        with("due", json!("2026-02-30"));
        with("rows", json!([{ "text": "x", "message": 10 }]));
        with("rows", json!([{ "text": "x".repeat(21), "message": 1 }]));
        with("rows", json!([{ "text": "x" }]));
        with(
            "rows",
            json!([{ "text": "x", "message": 1, "url": "https://example.com" }]),
        );
        with(
            "rows",
            json!([
                { "text": "a", "message": 1 },
                { "text": "b", "message": 2 },
                { "text": "c", "message": 3 }
            ]),
        );
        with("extra", json!("instructions"));
        for case in cases {
            assert!(ANSWER.check(case.clone()).is_err(), "{case}");
        }
        assert_eq!(
            ANSWER.check(json!({ "kind": "none", "due": "" })),
            Err(SchemaError::Missing("rows"))
        );
        assert_eq!(ANSWER.check(json!(["none"])), Err(SchemaError::NotAnObject));
    }

    #[test]
    fn rows_inside_rows_are_not_flat() {
        const DEEP: Schema = Schema {
            name: "deep",
            fields: &[Field {
                name: "outer",
                kind: Kind::Rows {
                    max: 1,
                    fields: &[Field {
                        name: "inner",
                        kind: Kind::Rows {
                            max: 1,
                            fields: ROW,
                        },
                    }],
                },
            }],
        };
        assert!(!DEEP.is_flat());
        assert!(
            DEEP.check(json!({ "outer": [{ "inner": [] }] })).is_err(),
            "a deep answer is never believed"
        );
    }
}
