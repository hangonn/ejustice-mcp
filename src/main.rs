use ejustice_mcp::mcp::EjusticeMcpServer;
use rmcp::ServiceExt;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::FmtSubscriber;

fn main() {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_max_level(LevelFilter::DEBUG)
            .finish(),
    )
    .unwrap();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = EjusticeMcpServer::new("https://ejustice.jud.na/ejustice");
            let service = server.serve(rmcp::transport::stdio()).await.unwrap();
            service.waiting().await.unwrap();
        });
}
