//! A toolbox over the Model Context Protocol.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

use crate::{Error, Toolbox};

/// A [`Toolbox`] as an MCP server: give it to [`rmcp::ServiceExt::serve`] with a transport.
#[derive(Clone)]
pub struct Server(Arc<Toolbox>);

impl Server {
    pub fn new(toolbox: Toolbox) -> Self {
        Self(Arc::new(toolbox))
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("fhir-toolbox", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .0
            .tools()
            .into_iter()
            .map(serde_json::from_value::<Tool>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        match self.0.call(&request.name, &arguments).await {
            Ok(value) => Ok(CallToolResult::structured(value).into()),
            Err(Error::UnknownTool(tool)) => Err(ErrorData::invalid_params(
                format!("no such tool: {tool}"),
                None,
            )),
            Err(error) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(error.to_string())]).into())
            }
        }
    }
}
