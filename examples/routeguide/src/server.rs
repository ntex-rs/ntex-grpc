//! RouteGuide server, shows unary, server, client and bidirectional
//! streaming methods.
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, convert::Infallible, time::Instant};

use ntex::{ServiceFactory, SharedCfg, server::Server};
use ntex_grpc::server::{GrpcServer, ServerError, ServerRequest, ServerResponse};
use ntex_grpc::{RecvStream, SendStream, server};

mod routeguide;
use crate::routeguide::{Feature, Point, Rectangle, RouteNote, RouteSummary};

/// Notes sent to `RouteChat`, by location.
type Notes = Arc<Mutex<HashMap<(i32, i32), Vec<RouteNote>>>>;

#[derive(Clone)]
struct RouteGuideServer {
    features: Arc<[Feature]>,
    notes: Notes,
}

#[server(crate::routeguide::RouteGuide)]
impl RouteGuideServer {
    /// Unary call, returns the feature at the point, a feature without a
    /// name if there is none.
    #[method(GetFeature)]
    async fn get_feature(&self, point: Point) -> Feature {
        self.feature_at(&point).cloned().unwrap_or(Feature {
            name: "".into(),
            location: point,
        })
    }

    /// Server streaming, sends the features in the rectangle.
    #[method(ListFeatures)]
    async fn list_features(&self, rect: Rectangle, tx: SendStream<Feature>) {
        for feature in self.features.iter() {
            if in_range(&feature.location, &rect) && tx.send(feature).await.is_err() {
                // the client is gone
                return;
            }
        }
    }

    /// Client streaming, sums up the route once the client closes its
    /// stream, or at the first error of the stream.
    #[method(RecordRoute)]
    async fn record_route(&self, mut rx: RecvStream<Point>) -> RouteSummary {
        let start = Instant::now();
        let mut summary = RouteSummary::default();
        let mut last: Option<Point> = None;

        while let Some(Ok(point)) = rx.recv().await {
            summary.point_count += 1;
            if self.feature_at(&point).is_some() {
                summary.feature_count += 1;
            }
            if let Some(last) = last {
                summary.distance += distance(&last, &point);
            }
            last = Some(point);
        }
        summary.elapsed_time = i32::try_from(start.elapsed().as_secs()).unwrap_or(i32::MAX);
        summary
    }

    /// Bidirectional streaming, sends back the notes sent earlier at the
    /// location of every received note.
    #[method(RouteChat)]
    async fn route_chat(
        &self,
        mut rx: RecvStream<RouteNote>,
        tx: SendStream<RouteNote>,
    ) -> Result<(), ServerError> {
        while let Some(note) = rx.recv().await {
            let note = note?;
            let key = (note.location.latitude, note.location.longitude);
            let prev = {
                let mut notes = self.notes.lock().unwrap();
                let prev = notes.get(&key).cloned().unwrap_or_default();
                notes.entry(key).or_default().push(note);
                prev
            };
            for note in &prev {
                tx.send(note).await?;
            }
        }
        Ok(())
    }
}

impl RouteGuideServer {
    fn feature_at(&self, point: &Point) -> Option<&Feature> {
        self.features
            .iter()
            .find(|f| f.location == *point && !f.name.is_empty())
    }
}

impl ServiceFactory<(), ServerRequest> for RouteGuideServer {
    type Res = ServerResponse;
    type Error = ServerError;
    type Service = RouteGuideServer;
    type InitError = Infallible;

    async fn create(&self, (): &()) -> Result<Self::Service, Self::InitError> {
        Ok(self.clone())
    }
}

fn in_range(point: &Point, rect: &Rectangle) -> bool {
    let (lo, hi) = (&rect.lo, &rect.hi);
    let lat = lo.latitude.min(hi.latitude)..=lo.latitude.max(hi.latitude);
    let lon = lo.longitude.min(hi.longitude)..=lo.longitude.max(hi.longitude);
    lat.contains(&point.latitude) && lon.contains(&point.longitude)
}

/// Distance between two points in metres, with the haversine formula.
fn distance(p1: &Point, p2: &Point) -> i32 {
    const COORD_FACTOR: f64 = 1e7;
    const R: f64 = 6_371_000.0; // earth radius in metres

    let lat1 = (f64::from(p1.latitude) / COORD_FACTOR).to_radians();
    let lat2 = (f64::from(p2.latitude) / COORD_FACTOR).to_radians();
    let lon1 = (f64::from(p1.longitude) / COORD_FACTOR).to_radians();
    let lon2 = (f64::from(p2.longitude) / COORD_FACTOR).to_radians();

    let a = ((lat2 - lat1) / 2.0).sin().powi(2)
        + lat1.cos() * lat2.cos() * ((lon2 - lon1) / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    (R * c) as i32
}

/// A few features to look up.
fn features() -> Arc<[Feature]> {
    [
        (
            "Patriots Path, Mendham, NJ 07945, USA",
            407_838_351,
            -746_143_763,
        ),
        (
            "101 New Jersey 10, Whippany, NJ 07981, USA",
            408_122_808,
            -743_999_179,
        ),
        ("U.S. 6, Shohola, PA 18458, USA", 413_628_156, -749_015_468),
        (
            "5 Conners Road, Kingston, NY 12401, USA",
            419_999_544,
            -740_371_136,
        ),
        (
            "Mid Hudson Psychiatric Center, New Hampton, NY 10958, USA",
            414_008_389,
            -743_951_297,
        ),
        ("", 415_464_102, -747_175_374),
    ]
    .into_iter()
    .map(|(name, latitude, longitude)| Feature {
        name: name.into(),
        location: Point {
            latitude,
            longitude,
        },
    })
    .collect()
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    let _ = env_logger::try_init();

    let matches = clap::Command::new("routeguide server")
        .arg(clap::arg!([addr] "Address to listen on").default_value("127.0.0.1:50051"))
        .get_matches();
    let addr = matches.get_one::<String>("addr").unwrap().clone();

    let svc = RouteGuideServer {
        features: features(),
        notes: Arc::default(),
    };

    println!("RouteGuide server listening on {addr}");
    Server::builder()
        .bind(
            "routeguide",
            addr,
            SharedCfg::new("GRPC"),
            async move |_| GrpcServer::new(svc.clone()),
        )?
        .workers(1)
        .run()
        .await
}
