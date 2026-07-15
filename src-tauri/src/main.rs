// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == "--mcp-stdio") {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build Odo MCP runtime");
        let result = runtime.block_on(async {
            match odo_tasks_lib::mcp::default_database_path() {
                Ok(path) => odo_tasks_lib::mcp::run_stdio(path).await,
                Err(error) => Err(error),
            }
        });
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    odo_tasks_lib::run()
}
