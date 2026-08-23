use clap::{Parser, Subcommand};
use flashvault::{Cache, SystemClock};
use std::io::{self, BufRead, Write};

#[derive(Parser)]
#[command(name = "flashvault", version, about = "An in-memory LRU cache with TTL expiry.")]
struct Cli {
    /// Max entries held at once.
    #[arg(short, long, default_value_t = 128)]
    capacity: usize,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Set a key, run one get against it, and print the result.
    Set {
        key: String,
        value: String,
        /// Time to live in milliseconds. Omit for no expiry.
        #[arg(long)]
        ttl_ms: Option<u64>,
    },
    /// Get a key and print the value or "(miss)".
    Get { key: String },
    /// Run a canned workload and print the resulting stats.
    Demo,
    /// Interactive read-eval-print loop: set/get/del/stats/keys/quit.
    Repl,
}

fn main() {
    let cli = Cli::parse();
    let mut cache: Cache<String, SystemClock> = Cache::new(cli.capacity.max(1), SystemClock);

    match cli.command {
        Some(Command::Set { key, value, ttl_ms }) => {
            cache.put(key.clone(), value, ttl_ms);
            println!("set {key}");
        }
        Some(Command::Get { key }) => match cache.get(&key) {
            Some(v) => println!("{v}"),
            None => println!("(miss)"),
        },
        Some(Command::Demo) => run_demo(cli.capacity.max(1)),
        Some(Command::Repl) | None => run_repl(cache),
    }
}

/// A single-process workload that exercises capacity eviction, TTL expiry,
/// and recency, then prints the stats it produced.
fn run_demo(capacity: usize) {
    let mut cache: Cache<String, SystemClock> = Cache::new(capacity, SystemClock);
    println!("flashvault demo, capacity={capacity}");

    for i in 0..capacity {
        cache.put(format!("key{i}"), format!("value{i}"), None);
    }
    println!("filled cache to capacity ({} entries)", cache.len());

    if let Some(v) = cache.get("key0") {
        println!("touched key0 -> {v} (now most-recently-used)");
    }

    cache.put("overflow".to_string(), "pushed out the LRU entry".to_string(), None);
    println!(
        "inserted one more key, size is still {} (<= capacity {})",
        cache.len(),
        cache.capacity()
    );

    cache.put("short-lived".to_string(), "gone soon".to_string(), Some(1));
    std::thread::sleep(std::time::Duration::from_millis(5));
    match cache.get("short-lived") {
        Some(_) => println!("short-lived key unexpectedly still present"),
        None => println!("short-lived key expired as expected"),
    }

    let stats = cache.stats();
    println!(
        "stats: hits={} misses={} evictions={} expirations={} size={} hit_rate={:.2}",
        stats.hits,
        stats.misses,
        stats.evictions,
        stats.expirations,
        stats.size,
        stats.hit_rate()
    );
}

fn run_repl(mut cache: Cache<String, SystemClock>) {
    println!("flashvault repl. commands: set <k> <v> [ttl_ms], get <k>, del <k>, keys, stats, quit");
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        let _ = stdout.flush();

        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        match parts.as_slice() {
            [] => continue,
            ["quit"] | ["exit"] => break,
            ["set", key, value] => {
                cache.put(key.to_string(), value.to_string(), None);
                println!("ok");
            }
            ["set", key, value, ttl] => match ttl.parse::<u64>() {
                Ok(ttl_ms) => {
                    cache.put(key.to_string(), value.to_string(), Some(ttl_ms));
                    println!("ok");
                }
                Err(_) => println!("error: ttl must be a non-negative integer of milliseconds"),
            },
            ["get", key] => match cache.get(key) {
                Some(v) => println!("{v}"),
                None => println!("(miss)"),
            },
            ["del", key] => {
                if cache.remove(key) {
                    println!("ok");
                } else {
                    println!("(not found)");
                }
            }
            ["keys"] => {
                for k in cache.keys_by_recency() {
                    println!("{k}");
                }
            }
            ["stats"] => {
                let s = cache.stats();
                println!(
                    "hits={} misses={} evictions={} expirations={} size={} hit_rate={:.2}",
                    s.hits, s.misses, s.evictions, s.expirations, s.size, s.hit_rate()
                );
            }
            _ => println!("error: unrecognized command"),
        }
    }
}
