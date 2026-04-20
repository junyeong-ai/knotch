//! Declarative frontmatter validation.

use std::collections::BTreeMap;

use compact_str::CompactString;
use serde_json::Value;

/// Value kinds a frontmatter field may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldType {
    /// Arbitrary UTF-8 string.
    String,
    /// Integer (serde_json Number with integer value).
    Integer,
    /// Boolean.
    Boolean,
    /// Value must equal one of the enumerated options.
    Enum(Vec<CompactString>),
    /// Array of strings.
    StringArray,
}

/// Validation rule for one field.
#[derive(Debug, Clone)]
pub struct FieldSchema {
    name: CompactString,
    ty: FieldType,
    required: bool,
}

impl FieldSchema {
    /// Build a required field of the given type.
    #[must_use]
    pub fn required(name: impl Into<CompactString>, ty: FieldType) -> Self {
        Self { name: name.into(), ty, required: true }
    }

    /// Build an optional field of the given type.
    #[must_use]
    pub fn optional(name: impl Into<CompactString>, ty: FieldType) -> Self {
        Self { name: name.into(), ty, required: false }
    }

    /// Field name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
}

/// A complete frontmatter schema.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterSchema {
    fields: BTreeMap<CompactString, FieldSchema>,
}

impl FrontmatterSchema {
    /// Start a new schema builder.
    #[must_use]
    pub fn builder() -> Self {
        Self::default()
    }

    /// Add a field to the schema.
    #[must_use]
    pub fn field(mut self, schema: FieldSchema) -> Self {
        self.fields.insert(schema.name.clone(), schema);
        self
    }

    /// Validate a parsed frontmatter object.
    ///
    /// # Errors
    /// Returns `SchemaError::MissingField` for absent required fields;
    /// `SchemaError::WrongType` for type mismatches;
    /// `SchemaError::Unknown` for fields not declared in the schema.
    pub fn validate(
        &self,
        object: &serde_json::Map<String, Value>,
    ) -> Result<(), SchemaError> {
        for schema in self.fields.values() {
            let value = object.get(schema.name.as_str());
            match (value, schema.required) {
                (None, true) => {
                    return Err(SchemaError::MissingField {
                        field: schema.name.clone(),
                    });
                }
                (None, false) => continue,
                (Some(v), _) => validate_type(schema.name.as_str(), &schema.ty, v)?,
            }
        }
        for field in object.keys() {
            if !self.fields.contains_key(field.as_str()) {
                return Err(SchemaError::Unknown { field: field.clone().into() });
            }
        }
        Ok(())
    }
}

fn validate_type(name: &str, ty: &FieldType, value: &Value) -> Result<(), SchemaError> {
    let ok = match (ty, value) {
        (FieldType::String, Value::String(_)) => true,
        (FieldType::Integer, Value::Number(n)) => n.is_i64() || n.is_u64(),
        (FieldType::Boolean, Value::Bool(_)) => true,
        (FieldType::StringArray, Value::Array(items)) => {
            items.iter().all(|i| matches!(i, Value::String(_)))
        }
        (FieldType::Enum(allowed), Value::String(s)) => {
            allowed.iter().any(|opt| opt == s)
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(SchemaError::WrongType {
            field: name.to_owned().into(),
            expected: format!("{ty:?}").into(),
        })
    }
}

/// Errors raised by schema validation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaError {
    /// A required field was absent.
    #[error("missing required field {field:?}")]
    MissingField {
        /// Field name.
        field: CompactString,
    },
    /// A field held the wrong type.
    #[error("field {field:?} has wrong type — expected {expected}")]
    WrongType {
        /// Field name.
        field: CompactString,
        /// Expected type description.
        expected: CompactString,
    },
    /// A field is not declared by the schema.
    #[error("field {field:?} is not declared by the schema")]
    Unknown {
        /// Field name.
        field: CompactString,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn spec_schema() -> FrontmatterSchema {
        FrontmatterSchema::builder()
            .field(FieldSchema::required("id", FieldType::String))
            .field(FieldSchema::required(
                "status",
                FieldType::Enum(vec!["draft".into(), "planning".into(), "archived".into()]),
            ))
            .field(FieldSchema::optional("tags", FieldType::StringArray))
    }

    #[test]
    fn accepts_valid_object() {
        let obj = json!({ "id": "sp-001", "status": "planning", "tags": ["a", "b"] });
        spec_schema()
            .validate(obj.as_object().expect("obj"))
            .expect("valid");
    }

    #[test]
    fn rejects_missing_required_field() {
        let obj = json!({ "status": "draft" });
        let err = spec_schema().validate(obj.as_object().expect("obj")).unwrap_err();
        assert_eq!(err, SchemaError::MissingField { field: "id".into() });
    }

    #[test]
    fn rejects_wrong_type() {
        let obj = json!({ "id": 123, "status": "draft" });
        let err = spec_schema().validate(obj.as_object().expect("obj")).unwrap_err();
        assert!(matches!(err, SchemaError::WrongType { .. }));
    }

    #[test]
    fn rejects_non_enum_value() {
        let obj = json!({ "id": "x", "status": "bogus" });
        let err = spec_schema().validate(obj.as_object().expect("obj")).unwrap_err();
        assert!(matches!(err, SchemaError::WrongType { .. }));
    }

    #[test]
    fn rejects_unknown_field() {
        let obj = json!({ "id": "x", "status": "draft", "extra": true });
        let err = spec_schema().validate(obj.as_object().expect("obj")).unwrap_err();
        assert_eq!(err, SchemaError::Unknown { field: "extra".into() });
    }

    #[test]
    fn accepts_optional_field_absence() {
        let obj = json!({ "id": "x", "status": "draft" });
        spec_schema().validate(obj.as_object().expect("obj")).expect("valid");
    }
}
