//! One paid Codex GraphQL request, printed raw: `cargo run --example gql -- '<query>'`.
//! Reads CODEX_MPP_KEY from the environment. Costs about $0.001.
use mpp::client::{Fetch, TempoProvider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let signer: mpp::PrivateKeySigner = std::env::var("CODEX_MPP_KEY")?.trim().parse()?;
    let provider =
        TempoProvider::new(signer, "https://rpc.tempo.xyz")?.with_client_id("hoodit-dev");
    let query = std::env::args().nth(1).expect("query argument");
    let started = std::time::Instant::now();
    let response = reqwest::Client::new()
        .post("https://graph.codex.io/graphql")
        .header("X-Codex-Payment", "mpp")
        .json(&serde_json::json!({ "query": query }))
        .send_with_payment(&provider)
        .await?;
    eprintln!(
        "{} in {:.1}s",
        response.status(),
        started.elapsed().as_secs_f64()
    );
    println!("{}", response.text().await?);
    Ok(())
}
