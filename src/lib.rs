//! FHIR tools for agents, offline.
//!
//! A [`Toolbox`] holds the FHIR packages it was given. [`Toolbox::tools`] says what it can do, as
//! the tool definitions of the Model Context Protocol, and [`Toolbox::call`] does it. Nothing
//! but [`mcp::Server`] depends on a transport, so a FHIR server can offer the same tools next to
//! its own.

mod error;
pub mod mcp;
mod packages;
mod tools;

use std::collections::HashMap;
use std::sync::Arc;

use octofhir_fhirpath::{FhirPathEngine, evaluator::create_function_registry};
use octofhir_fhirschema::{
    FhirSchema, FhirSchemaModelProvider, FhirValidator, FhirVersion, InMemorySchemaProvider,
    ModelFhirVersion, get_schemas,
};

pub use error::Error;
pub use packages::Definition;

/// The FHIR packages a toolbox holds, and what works on them.
pub struct Toolbox {
    definitions: Vec<Definition>,
    packages: Vec<String>,
    engine: Arc<FhirPathEngine>,
    validator: FhirValidator,
    model: Arc<FhirSchemaModelProvider>,
}

/// What to load into a [`Toolbox`].
pub struct Builder {
    version: FhirVersion,
    packages: Vec<String>,
}

impl Toolbox {
    /// A toolbox for `version` holding only the core of that version.
    pub fn builder(version: FhirVersion) -> Builder {
        Builder {
            version,
            packages: Vec::new(),
        }
    }

    /// The packages loaded, as `name@version`.
    pub fn packages(&self) -> &[String] {
        &self.packages
    }
}

impl Builder {
    /// Load a package: `name@version`, installed from the registry when missing, or a folder.
    pub fn package(mut self, spec: impl Into<String>) -> Self {
        self.packages.push(spec.into());
        self
    }

    /// Read the packages and prepare the validator and the FHIRPath engine.
    pub async fn build(self) -> Result<Toolbox, Error> {
        let specs = self.packages;
        let (definitions, packages) = tokio::task::spawn_blocking(move || packages::load(&specs))
            .await
            .map_err(|error| Error::Package(error.to_string()))??;

        let mut schemas: HashMap<String, FhirSchema> = get_schemas(self.version).clone();
        for definition in &definitions {
            // By URL alone: a definition's name can be that of a core type, as
            // `ParameterDefinition` is the name of an extension too.
            if let Some(schema) = packages::schema_of(definition) {
                schemas.insert(schema.url.clone(), schema);
            }
        }
        let shared: HashMap<String, Arc<FhirSchema>> = schemas
            .iter()
            .map(|(name, schema)| (name.clone(), Arc::new(schema.clone())))
            .collect();
        let model = Arc::new(FhirSchemaModelProvider::new(
            schemas,
            model_version(self.version),
        ));
        let engine = Arc::new(
            FhirPathEngine::new(Arc::new(create_function_registry()), model.clone())
                .await
                .map_err(|error| Error::Engine(error.to_string()))?,
        );
        let validator = FhirValidator::new_with_fhirpath(
            Arc::new(InMemorySchemaProvider::from_map(shared)),
            engine.clone(),
        );
        Ok(Toolbox {
            definitions,
            packages,
            engine,
            validator,
            model,
        })
    }
}

fn model_version(version: FhirVersion) -> ModelFhirVersion {
    match version {
        FhirVersion::R4 => ModelFhirVersion::R4,
        FhirVersion::R4B => ModelFhirVersion::R4B,
        FhirVersion::R5 => ModelFhirVersion::R5,
        FhirVersion::R6 => ModelFhirVersion::R6,
    }
}
