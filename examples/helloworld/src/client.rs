#![allow(deprecated, clippy::never_loop, clippy::expect_fun_call)]
use std::{sync::atomic::AtomicUsize, sync::atomic::Ordering, sync::Arc, thread, time::Duration};

use ntex::{rt::spawn, rt::System};
use ntex_grpc::client::Connector;

mod helloworld;
use self::helloworld::{greeter_client::Greeter, HelloRequest};

fn main() {
    std::env::set_var("RUST_LOG", "trace");
    env_logger::init();

    let matches = clap::App::new("helloworld client")
        .version("0.1")
        .about("Applies load to helloworld server")
        .args_from_usage(
            "<ip> 'Helloworld server address'
                -r, --report-rate=[SECONDS] 'seconds between average reports'
                -c, --concurrency=[NUMBER] 'number of client connections to open and use concurrently for sending'
                -t, --threads=[NUMBER] 'number of threads to use'",
        )
        .get_matches();

    let ip = matches.value_of("ip").unwrap().to_owned();

    let threads = parse_u64_default(matches.value_of("threads"), num_cpus::get() as u64);
    let concurrency = parse_u64_default(matches.value_of("concurrency"), 1);
    let report_rate = parse_u64_default(matches.value_of("report-rate"), 1) as usize;
    let perf_counters = Arc::new(PerfCounters::default());

    for t in 0..threads {
        let addr = ip.clone();
        let counters = perf_counters.clone();

        thread::spawn(move || {
            println!("Starting thread: {}", t);
            let sys = System::new("client");

            let _ = sys.block_on(async move {
                let connector = Connector::default();
                let client: Greeter<_> = connector.create(addr.clone()).await.unwrap();

                for _ in 0..concurrency - 1 {
                    let cnt = counters.clone();
                    let client = client.clone();
                    spawn(async move {
                        loop {
                            client
                                .say_hello(&HelloRequest {
                                    name: "world".into(),
                                })
                                .await
                                .unwrap();
                            cnt.register_request();
                        }
                    });
                }

                loop {
                    client
                        .say_hello(&HelloRequest {
                            name: "world".into(),
                        })
                        .await
                        .unwrap();
                    counters.register_request();
                    break;
                }
            });
        });
    }

    start_report_thread(perf_counters, report_rate, threads, concurrency);
}

fn parse_u64_default(input: Option<&str>, default: u64) -> u64 {
    input
        .map(|v| v.parse().expect(&format!("not a valid number: {}", v)))
        .unwrap_or(default)
}

fn start_report_thread(
    counters: Arc<PerfCounters>,
    report_rate_secs: usize,
    threads: u64,
    conns: u64,
) {
    let _ = thread::spawn(move || {
        let delay = Duration::new(report_rate_secs as u64, 0);
        loop {
            thread::sleep(delay);

            let req_count = counters.pull_request_count();
            if req_count != 0 {
                let latency = counters.pull_latency_ns();
                let latency_max = counters.pull_latency_max_ns();
                println!(
                    "rate: {}, threads: {}, connections: {}, latency: {:?}, latency max: {:?}",
                    req_count / report_rate_secs,
                    threads,
                    conns,
                    Duration::from_nanos(latency / req_count as u64),
                    Duration::from_nanos(latency_max as u64)
                );
            }
        }
    })
    .join();
}

pub struct PerfCounters {
    req: AtomicUsize,
    conn: AtomicUsize,
    lat: AtomicUsize,
    lat_max: AtomicUsize,
}

impl Default for PerfCounters {
    fn default() -> PerfCounters {
        PerfCounters {
            req: AtomicUsize::new(0),
            conn: AtomicUsize::new(0),
            lat: AtomicUsize::new(0),
            lat_max: AtomicUsize::new(0),
        }
    }
}

impl PerfCounters {
    pub fn pull_request_count(&self) -> usize {
        self.req.swap(0, Ordering::SeqCst)
    }

    pub fn pull_connections_count(&self) -> usize {
        self.conn.swap(0, Ordering::SeqCst)
    }

    pub fn pull_latency_ns(&self) -> u64 {
        self.lat.swap(0, Ordering::SeqCst) as u64
    }

    pub fn pull_latency_max_ns(&self) -> u64 {
        self.lat_max.swap(0, Ordering::SeqCst) as u64
    }

    pub fn register_request(&self) {
        self.req.fetch_add(1, Ordering::SeqCst);
    }

    pub fn register_connection(&self) {
        self.conn.fetch_add(1, Ordering::SeqCst);
    }

    pub fn register_latency(&self, nanos: u64) {
        let nanos = nanos as usize;
        self.lat.fetch_add(nanos, Ordering::SeqCst);
        loop {
            let current = self.lat_max.load(Ordering::SeqCst);
            if current >= nanos
                || self
                    .lat_max
                    .compare_and_swap(current, nanos, Ordering::SeqCst)
                    == current
            {
                break;
            }
        }
    }
}
