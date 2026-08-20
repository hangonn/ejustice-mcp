use ejustice_mcp_core::server::EjusticeMcpServer;
use rmcp::ServiceExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::FmtSubscriber;

fn main() {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_max_level(LevelFilter::DEBUG)
            .finish(),
    )
    .expect("Failed to install the global tracing subscriber");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build the Tokio runtime")
        .block_on(async {
            let base_url = std::env::var("EJUSTICE_BASE_URL")
                .unwrap_or_else(|_| "https://ejustice.jud.na/ejustice".to_string());

            let server = EjusticeMcpServer::new(base_url);

            let service = server
                .serve(rmcp::transport::stdio())
                .await
                .expect("Failed to start the MCP stdio transport");

            service
                .waiting()
                .await
                .expect("MCP service exited with an error");
        });
}
