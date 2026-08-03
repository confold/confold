use crate::{InputRole, SemanticBundle, SCHEMA_VERSION};
use serde_json::{json, Value};

pub fn proposal_json_schema(bundle: &SemanticBundle) -> Value {
    let roles = bundle
        .inputs
        .iter()
        .map(|input| match input.role {
            InputRole::Left => "left",
            InputRole::Right => "right",
            InputRole::Base => "base",
        })
        .collect::<Vec<_>>();
    json!({
        "type": "object",
        "properties": {
            "schema_version": {
                "type": "integer",
                "const": SCHEMA_VERSION
            },
            "operation_id": {
                "type": "string",
                "const": bundle.operation_id
            },
            "verdict": {
                "type": "string",
                "enum": ["equivalent", "prefer_left", "prefer_right", "merged", "uncertain"]
            },
            "summary": {
                "type": "string",
                "minLength": 1
            },
            "contributions": {
                "type": "array",
                "minItems": 2,
                "items": {
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "enum": roles
                        },
                        "intent": {
                            "type": "string",
                            "minLength": 1
                        },
                        "disposition": {
                            "type": "string",
                            "enum": [
                                "preserved",
                                "already_present",
                                "superseded",
                                "omitted",
                                "uncertain"
                            ]
                        }
                    },
                    "required": ["source", "intent", "disposition"],
                    "additionalProperties": false
                }
            },
            "warnings": {
                "type": "array",
                "items": {
                    "type": "string"
                }
            },
            "result": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "null" }
                ]
            }
        },
        "required": [
            "schema_version",
            "operation_id",
            "verdict",
            "summary",
            "contributions",
            "warnings",
            "result"
        ],
        "allOf": [
            {
                "if": {
                    "properties": {
                        "verdict": { "const": "merged" }
                    }
                },
                "then": {
                    "properties": {
                        "result": { "type": "string", "minLength": 1 }
                    }
                }
            },
            {
                "if": {
                    "properties": {
                        "verdict": {
                            "enum": ["equivalent", "prefer_left", "prefer_right", "uncertain"]
                        }
                    }
                },
                "then": {
                    "properties": {
                        "result": { "type": "null" }
                    }
                }
            }
        ],
        "additionalProperties": false
    })
}
