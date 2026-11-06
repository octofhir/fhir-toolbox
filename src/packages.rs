//! The StructureDefinitions of the packages a toolbox holds.

use fhir_diff::package::{self, Package};
use octofhir_fhirschema::{FhirSchema, StructureDefinition, translate};
use serde_json::Value;

use crate::Error;

/// A StructureDefinition and the package it came from.
#[derive(Debug, Clone)]
pub struct Definition {
    pub package: String,
    pub json: Value,
}

impl Definition {
    fn field(&self, name: &str) -> &str {
        self.json[name].as_str().unwrap_or_default()
    }

    pub fn url(&self) -> &str {
        self.field("url")
    }

    pub fn name(&self) -> &str {
        self.field("name")
    }

    /// The type it constrains: `Patient`, `Extension`.
    pub fn kind(&self) -> &str {
        self.field("type")
    }

    pub fn is_extension(&self) -> bool {
        self.kind() == "Extension" && self.field("derivation") == "constraint"
    }

    pub fn is_profile(&self) -> bool {
        self.kind() != "Extension" && self.field("derivation") == "constraint"
    }

    /// Whether every word of `query` is in its name, title, URL or description.
    pub fn matches(&self, query: &str) -> bool {
        let text = ["name", "title", "url", "description"]
            .map(|name| self.field(name).to_lowercase())
            .join(" ");
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }

    /// Where an extension may be used: the `expression` of each `context`.
    pub fn contexts(&self) -> Vec<&str> {
        self.json["context"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|context| context["expression"].as_str().or(context.as_str()))
            .collect()
    }
}

fn definitions(package: &Package) -> impl Iterator<Item = Definition> {
    let id = package.id();
    package
        .definitions
        .iter()
        .map(move |definition| Definition {
            package: id.clone(),
            json: definition.json.clone(),
        })
}

/// The definitions of `specs` and of the packages they depend on, and what was loaded.
pub(crate) fn load(specs: &[String]) -> Result<(Vec<Definition>, Vec<String>), Error> {
    let mut found = Vec::new();
    let mut loaded = Vec::new();
    for spec in specs {
        let package = package::load(spec).map_err(Error::Package)?;
        for one in std::iter::once(&package.package).chain(&package.dependencies) {
            let id = one.id();
            if !loaded.contains(&id) {
                found.extend(definitions(one));
                loaded.push(id);
            }
        }
    }
    Ok((found, loaded))
}

/// The FHIR Schema of a definition. One that does not convert is left out: it can still be found
/// and read, only not validated against.
pub(crate) fn schema_of(definition: &Definition) -> Option<FhirSchema> {
    let parsed: StructureDefinition = serde_json::from_value(definition.json.clone()).ok()?;
    translate(parsed, None).ok()
}
