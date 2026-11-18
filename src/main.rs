mod api;
mod app;
mod cache;
mod ui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = app::run(&mut terminal).await;
    ratatui::restore();
    result
}
