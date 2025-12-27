mod debug;
mod kiro;
mod model;
mod test;

use futures::StreamExt;

#[tokio::main]
async fn main() {
    // 初始化日志
    tracing_subscriber::fmt::init();

    if let Err(e) = test::call_stream_api().await {
        eprintln!("错误: {}", e);
        std::process::exit(1);
    }
}
