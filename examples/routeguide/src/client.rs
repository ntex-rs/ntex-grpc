//! RouteGuide client, calls every method of the RouteGuide server.
use std::{error::Error, time::Duration};

use ntex::{SharedCfg, time::sleep, util::join};
use ntex_grpc::{SendStream, StreamError, client::Client};
use ntex_h2::client as h2;

mod routeguide;
use crate::routeguide::{Point, Rectangle, RouteGuideClient, RouteNote};

type RouteGuide = RouteGuideClient<Client>;

#[ntex::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = env_logger::try_init();

    let matches = clap::Command::new("routeguide client")
        .arg(clap::arg!([addr] "RouteGuide server address").default_value("127.0.0.1:50051"))
        .get_matches();
    let addr = matches.get_one::<String>("addr").unwrap().clone();

    let h2client = h2::Client::builder(addr).build(SharedCfg::default());
    let client = RouteGuideClient::new(Client::new(h2client));

    get_feature(&client).await?;
    list_features(&client).await?;
    record_route(&client).await?;
    route_chat(&client).await?;
    Ok(())
}

/// Unary call.
async fn get_feature(client: &RouteGuide) -> Result<(), Box<dyn Error>> {
    println!("*** GetFeature");
    for point in [point(407_838_351, -746_143_763), point(0, 0)] {
        let feature = client.get_feature(&point).send().await?.output;
        if feature.name.is_empty() {
            println!("no feature at {point:?}");
        } else {
            println!("found {:?} at {point:?}", feature.name);
        }
    }
    Ok(())
}

/// Server streaming, receives the features in a rectangle.
async fn list_features(client: &RouteGuide) -> Result<(), Box<dyn Error>> {
    println!("*** ListFeatures");
    let rect = Rectangle {
        lo: point(400_000_000, -750_000_000),
        hi: point(420_000_000, -730_000_000),
    };
    let mut rx = client.list_features(&rect).open().await?;
    while let Some(feature) = rx.recv().await {
        println!("feature: {:?}", feature?);
    }
    println!("status: {:?}", rx.status());
    Ok(())
}

/// Client streaming, sends points while `send()` waits for the summary.
async fn record_route(client: &RouteGuide) -> Result<(), Box<dyn Error>> {
    println!("*** RecordRoute");
    let points = [
        point(407_838_351, -746_143_763),
        point(408_122_808, -743_999_179),
        point(413_628_156, -749_015_468),
        point(410_000_000, -745_000_000),
    ];

    let tx = SendStream::new();
    let (res, sent) = join(client.record_route(&tx).send(), async {
        for p in &points {
            println!("visiting {p:?}");
            tx.send(p).await?;
            sleep(Duration::from_millis(100)).await;
        }
        // the end of the stream, the server sends the summary
        tx.close();
        Ok::<_, StreamError>(())
    })
    .await;
    sent?;
    println!("summary: {:?}", res?.output);
    Ok(())
}

/// Bidirectional streaming, sends notes and receives the earlier notes at
/// the same locations.
async fn route_chat(client: &RouteGuide) -> Result<(), Box<dyn Error>> {
    println!("*** RouteChat");
    let notes = [
        note(0, 1, "First message"),
        note(0, 2, "Second message"),
        note(0, 1, "Third message"),
        note(0, 2, "Fourth message"),
    ];

    let (tx, mut rx) = client.route_chat().open().await?;
    let (sent, received) = join(
        async {
            for n in &notes {
                println!("sending {:?} at {:?}", n.message, n.location);
                tx.send(n).await?;
            }
            tx.close();
            Ok::<_, StreamError>(())
        },
        async {
            while let Some(n) = rx.recv().await {
                let n = n?;
                println!("got {:?} at {:?}", n.message, n.location);
            }
            Ok::<_, StreamError>(())
        },
    )
    .await;
    sent?;
    received?;
    println!("status: {:?}", rx.status());
    Ok(())
}

fn point(latitude: i32, longitude: i32) -> Point {
    Point {
        latitude,
        longitude,
    }
}

fn note(latitude: i32, longitude: i32, message: &str) -> RouteNote {
    RouteNote {
        location: point(latitude, longitude),
        message: message.into(),
    }
}
