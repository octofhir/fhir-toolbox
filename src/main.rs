//! `fhir-toolbox`: an MCP server on stdio.

use clap::{Parser, ValueEnum};
use fhir_toolbox::Toolbox;
use fhir_toolbox::mcp::Server;
use octofhir_fhirschema::FhirVersion;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

#[derive(Clone, Copy, ValueEnum)]
enum Version {
    R4,
    R4b,
    R5,
    R6,
}

/// FHIR tools for agents, offline, over the Model Context Protocol on stdio.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// A package: name@version, or a folder. Can be repeated.
    #[arg(long = "package", short)]
    packages: Vec<String>,
    /// The FHIR version of the core definitions.
    #[arg(long, value_enum, default_value = "r4")]
    fhir: Version,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let version = match cli.fhir {
        Version::R4 => FhirVersion::R4,
        Version::R4b => FhirVersion::R4B,
        Version::R5 => FhirVersion::R5,
        Version::R6 => FhirVersion::R6,
    };
    let mut builder = Toolbox::builder(version);
    for package in cli.packages {
        builder = builder.package(package);
    }
    let toolbox = match builder.build().await {
        Ok(toolbox) => toolbox,
        Err(error) => {
            eprintln!("error: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    eprintln!("loaded: {}", toolbox.packages().join(", "));
    let running = match Server::new(toolbox).serve(stdio()).await {
        Ok(running) => running,
        Err(error) => {
            eprintln!("error: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match running.waiting().await {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
