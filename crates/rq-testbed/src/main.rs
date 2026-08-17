//! `rq-testbed` — run the API the examples talk about.
//!
//! ```console
//! $ cargo run -p rq-testbed
//! rq-testbed listening on http://127.0.0.1:8087
//! ```

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "rq-testbed",
    version,
    about = "A small HTTP API to point rq at",
    long_about = "Serves the endpoints the rq examples and tests use: a login that hands \
                  out a token and a session cookie, a /me that accepts either, a list \
                  worth rendering, and an /echo that mirrors whatever you sent.\n\n\
                  Loopback only. Answers are deterministic."
)]
struct Cli {
    /// Port to listen on. 0 picks a free one.
    #[arg(short, long, default_value = "8087")]
    port: u16,

    /// Print the routes and exit.
    #[arg(long)]
    routes: bool,
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    if cli.routes {
        for (route, what) in rq_testbed::ROUTES {
            println!("{route:<34} {what}");
        }
        return Ok(());
    }

    let server = rq_testbed::Server::start(cli.port)?;
    println!("rq-testbed listening on {}", server.base_url);
    println!("  routes:  rq-testbed --routes");
    println!(
        "  try it:  rq r login --project examples/testbed --var host={}",
        server.base_url
    );
    println!("  stop it: ctrl-c");

    // The server owns its own threads; park this one so dropping it doesn't stop them.
    loop {
        std::thread::park();
    }
}
