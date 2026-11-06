//! The tools of a toolbox.

use octofhir_fhirpath::{Collection, EvaluationContext};
use octofhir_sof::ViewDefinition;
use serde_json::{Value, json};

use crate::packages::Definition;
use crate::{Error, Toolbox};

const DEFAULT_LIMIT: usize = 20;

fn string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, Error> {
    arguments[name]
        .as_str()
        .ok_or_else(|| Error::InvalidArguments(format!("{name} is required")))
}

fn limit(arguments: &Value) -> usize {
    arguments["limit"]
        .as_u64()
        .map_or(DEFAULT_LIMIT, |limit| limit as usize)
}

fn summary(definition: &Definition) -> Value {
    json!({
        "url": definition.url(),
        "name": definition.name(),
        "title": definition.json["title"],
        "type": definition.kind(),
        "package": definition.package,
    })
}

/// What a differential says of an element.
fn element(element: &Value) -> Value {
    let mut described = json!({"path": element["path"]});
    for name in [
        "sliceName",
        "short",
        "min",
        "max",
        "type",
        "mustSupport",
        "binding",
        "fixedCode",
        "patternCodeableConcept",
    ] {
        if !element[name].is_null() {
            described[name] = element[name].clone();
        }
    }
    described
}

impl Toolbox {
    /// The tools, as the `tools` of an MCP `tools/list` result.
    pub fn tools(&self) -> Vec<Value> {
        let text = |description: &str| json!({"type": "string", "description": description});
        let search = json!({"type": "object", "properties": {
            "query": text("Words to find in the name, title, URL or description"),
            "type": text("The resource type a profile constrains"),
            "limit": {"type": "integer", "description": "At most this many, 20 by default"},
        }});
        vec![
            json!({"name": "find_profiles", "title": "Find profiles",
                "description": "Find profiles in the loaded packages.",
                "annotations": {"readOnlyHint": true},
                "inputSchema": search}),
            json!({"name": "find_extensions", "title": "Find extensions",
            "description": "Find extensions in the loaded packages, with where they may be used.",
            "annotations": {"readOnlyHint": true},
            "inputSchema": {"type": "object", "properties": {
                "query": text("Words to find in the name, title, URL or description"),
                "context": text("An element the extension may be used on, such as Patient"),
                "limit": {"type": "integer", "description": "At most this many, 20 by default"},
            }}}),
            json!({"name": "describe_profile", "title": "Describe a profile",
            "description": "What a profile or extension changes: its differential, element by element.",
            "annotations": {"readOnlyHint": true},
            "inputSchema": {"type": "object", "required": ["profile"], "properties": {
                "profile": text("The canonical URL or name"),
            }}}),
            json!({"name": "validate_resource", "title": "Validate a resource",
            "description": "Validate a resource against FHIR, and a profile when one is given. Code bindings are not checked: that needs a terminology server.",
            "annotations": {"readOnlyHint": true},
            "inputSchema": {"type": "object", "required": ["resource"], "properties": {
                "resource": {"type": "object", "description": "The FHIR resource"},
                "profile": text("The canonical URL of a profile"),
            }}}),
            json!({"name": "evaluate_fhirpath", "title": "Evaluate FHIRPath",
            "description": "Evaluate a FHIRPath expression, on a resource when one is given.",
            "annotations": {"readOnlyHint": true},
            "inputSchema": {"type": "object", "required": ["expression"], "properties": {
                "expression": text("The FHIRPath expression"),
                "resource": {"type": "object", "description": "The FHIR resource"},
            }}}),
            json!({"name": "run_view", "title": "Run a view",
            "description": "Run a SQL on FHIR ViewDefinition over resources and return its rows.",
            "annotations": {"readOnlyHint": true},
            "inputSchema": {"type": "object", "required": ["view", "resources"], "properties": {
                "view": {"type": "object", "description": "The ViewDefinition"},
                "resources": {"type": "array", "items": {"type": "object"},
                    "description": "The resources to run it over"},
            }}}),
        ]
    }

    /// Run a tool.
    pub async fn call(&self, name: &str, arguments: &Value) -> Result<Value, Error> {
        match name {
            "find_profiles" => Ok(self.find_profiles(arguments)),
            "find_extensions" => Ok(self.find_extensions(arguments)),
            "describe_profile" => self.describe_profile(arguments),
            "validate_resource" => self.validate_resource(arguments).await,
            "evaluate_fhirpath" => self.evaluate_fhirpath(arguments).await,
            "run_view" => self.run_view(arguments).await,
            other => Err(Error::UnknownTool(other.to_owned())),
        }
    }

    fn find_profiles(&self, arguments: &Value) -> Value {
        let query = arguments["query"].as_str().unwrap_or_default();
        let kind = arguments["type"].as_str();
        let found: Vec<&Definition> = self
            .definitions
            .iter()
            .filter(|definition| definition.is_profile() && definition.matches(query))
            .filter(|definition| kind.is_none_or(|kind| definition.kind() == kind))
            .collect();
        json!({
            "total": found.len(),
            "profiles": found.iter().take(limit(arguments)).map(|d| summary(d)).collect::<Vec<_>>(),
        })
    }

    fn find_extensions(&self, arguments: &Value) -> Value {
        let query = arguments["query"].as_str().unwrap_or_default();
        let context = arguments["context"].as_str();
        let found: Vec<&Definition> =
            self.definitions
                .iter()
                .filter(|definition| definition.is_extension() && definition.matches(query))
                .filter(|definition| {
                    context.is_none_or(|context| {
                        definition.contexts().iter().any(|used| {
                            *used == context || used.starts_with(&format!("{context}."))
                        })
                    })
                })
                .collect();
        let described = found.iter().take(limit(arguments)).map(|definition| {
            let mut extension = summary(definition);
            extension["context"] = json!(definition.contexts());
            extension
        });
        json!({"total": found.len(), "extensions": described.collect::<Vec<_>>()})
    }

    fn definition(&self, profile: &str) -> Option<&Definition> {
        let canonical = profile.split('|').next().unwrap_or(profile);
        self.definitions
            .iter()
            .find(|definition| definition.url() == canonical || definition.name() == canonical)
    }

    fn describe_profile(&self, arguments: &Value) -> Result<Value, Error> {
        let profile = string(arguments, "profile")?;
        let definition = self
            .definition(profile)
            .ok_or_else(|| Error::Failed(format!("{profile} is in no loaded package")))?;
        let elements = definition.json["differential"]["element"]
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        let mut described = summary(definition);
        described["description"] = definition.json["description"].clone();
        described["baseDefinition"] = definition.json["baseDefinition"].clone();
        described["elements"] = elements.iter().map(element).collect();
        Ok(described)
    }

    async fn validate_resource(&self, arguments: &Value) -> Result<Value, Error> {
        let resource = &arguments["resource"];
        let kind = resource["resourceType"]
            .as_str()
            .ok_or_else(|| Error::InvalidArguments("resource has no resourceType".into()))?;
        let mut schemas = vec![kind.to_owned()];
        if let Some(profile) = arguments["profile"].as_str() {
            let canonical = profile.split('|').next().unwrap_or(profile);
            if self.definition(canonical).is_none() {
                return Err(Error::Failed(format!("{profile} is in no loaded package")));
            }
            schemas.push(canonical.to_owned());
        }
        let result = self.validator.validate(resource, schemas).await;
        serde_json::to_value(&result).map_err(|error| Error::Failed(error.to_string()))
    }

    async fn evaluate_fhirpath(&self, arguments: &Value) -> Result<Value, Error> {
        let expression = string(arguments, "expression")?;
        let input = match arguments.get("resource") {
            Some(resource) => {
                Collection::from_json_resource(resource.clone(), Some(self.model.clone()))
                    .await
                    .map_err(|error| Error::Failed(error.to_string()))?
            }
            None => Collection::empty(),
        };
        let context = EvaluationContext::new(input, self.model.clone(), None, None, None);
        let result = self
            .engine
            .evaluate(expression, &context)
            .await
            .map_err(|error| Error::Failed(error.to_string()))?;
        let values: Vec<Value> = result
            .value
            .iter()
            .map(|item| item.to_json_value())
            .collect();
        Ok(json!({"result": values}))
    }

    async fn run_view(&self, arguments: &Value) -> Result<Value, Error> {
        let view = ViewDefinition::from_json(&arguments["view"])
            .map_err(|error| Error::InvalidArguments(error.to_string()))?;
        let resources = arguments["resources"]
            .as_array()
            .ok_or_else(|| Error::InvalidArguments("resources is required".into()))?;
        let result = octofhir_sof::execute(&view, resources)
            .await
            .map_err(|error| Error::Failed(error.to_string()))?;
        let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
        let rows: Vec<Value> = result
            .data
            .iter()
            .map(|row| {
                names
                    .iter()
                    .zip(row)
                    .map(|(name, value)| ((*name).to_owned(), value.clone()))
                    .collect()
            })
            .collect();
        Ok(json!({"columns": names, "rows": rows}))
    }
}
