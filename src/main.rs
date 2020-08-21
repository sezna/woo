use bytes::buf::BufExt;
use futures_util::{stream, StreamExt};
use hyper::client::HttpConnector;
use hyper::service::{make_service_fn, service_fn};
use hyper::{header, Body, Client, Method, Request, Response, Server, StatusCode};
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use sqlx::prelude::*;
use sqlx_pg_migrate::migrate;
use std::env;

type GenericError = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, GenericError>;

static INDEX: &[u8] = include_bytes!("html/index.html");
static INTERNAL_SERVER_ERROR: &[u8] = b"Internal Server Error";
static NOTFOUND: &[u8] = b"Not Found";
static POST_DATA: &str = r#"{"original": "data"}"#;
static URL: &str = "http://127.0.0.1:1337/json_api";

async fn client_request_response(client: &Client<HttpConnector>) -> Result<Response<Body>> {
    let req = Request::builder()
        .method(Method::POST)
        .uri(URL)
        .header(header::CONTENT_TYPE, "application/json")
        .body(POST_DATA.into())
        .unwrap();

    let web_res = client.request(req).await?;
    // Compare the JSON we sent (before) with what we received (after):
    let before = stream::once(async {
        Ok(format!(
            "<b>POST request body</b>: {}<br><b>Response</b>: ",
            POST_DATA,
        )
        .into())
    });
    let after = web_res.into_body();
    let body = Body::wrap_stream(before.chain(after));

    Ok(Response::new(body))
}

#[derive(Deserialize, Serialize)]
struct NearbyRestaurantsRequest {
    latitude: String,
    longitude: String,
}

#[derive(Deserialize, Serialize)]
struct PlacesNearbySearchResponse {
    next_page_token: String,
    results: Vec<PlacesListing>,
}

#[derive(Deserialize, Serialize)]
struct PlacesListing {
    business_status: String,
    geometry: PlacesLocation,
    name: String,
    place_id: String,
    reference: String,
    types: Vec<String>,
    vicinity: String,
}

#[derive(Deserialize, Serialize)]
struct PlacesLocation {
    location: LatLong,
    viewport: Viewport,
}

#[derive(Deserialize, Serialize)]
struct Viewport {
    northeast: LatLong,
    southwest: LatLong,
}

#[derive(Deserialize, Serialize)]
struct LatLong {
    #[serde(rename = "lat")]
    latitude: f32,
    #[serde(rename = "lng")]
    longitude: f32,
}

async fn nearby_restaurants(req: Request<Body>, pool: &PgPool) -> Result<Response<Body>> {
    let api_key = env::var("GOOGLE_PLACES_API_KEY")?;
    // Aggregate the body...
    let whole_body = hyper::body::aggregate(req).await?;

    let NearbyRestaurantsRequest {
        latitude,
        longitude,
    } = serde_json::from_reader(whole_body.reader())?;
    let query_url = format!("https://maps.googleapis.com/maps/api/place/nearbysearch/json?key={}&location={},{}&rankby=distance&type=restaurant", api_key, latitude, longitude );
    let body = reqwest::get(&query_url).await?.bytes().await?;
    let places_response: PlacesNearbySearchResponse = serde_json::from_slice(&body)?;

    // now, insert the places into the db
    let values_string = places_response
        .results
        .iter()
        .enumerate()
        .map(|(idx, x)| {
            format!(
                "(\'{}\', \'{}\', \'{}\', \'{}\', {}, \'{}\', {}, {}, {}, {}, {}, {})",
                x.business_status,
                x.name.replace("'", "''"),
                x.place_id,
                x.reference,
                format!(
                    "\'{{{}}}\'",
                    x.types
                        .iter()
                        .map(|y| format!("\"{}\"", y.replace("'", "\\'")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                x.vicinity,
                x.geometry.location.latitude,
                x.geometry.location.longitude,
                x.geometry.viewport.northeast.latitude,
                x.geometry.viewport.northeast.longitude,
                x.geometry.viewport.southwest.latitude,
                x.geometry.viewport.southwest.longitude
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let query_string = format!(
        "
            INSERT INTO places (
               business_status,
               name,
               place_id,
               reference,
               types,
               vicinity,
               location_latitude,
               location_longitude,
               viewport_northeast_latitude,
               viewport_northeast_longitude,
               viewport_southwest_latitude,
               viewport_southwest_longitude 
            ) VALUES {}
            ON CONFLICT DO NOTHING;
        ",
        values_string
    );

    let _res = sqlx::query(&query_string).execute(pool).await?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&places_response)?))?;
    Ok(response)
}

async fn router(
    req: Request<Body>,
    client: Client<HttpConnector>,
    pool: PgPool,
) -> Result<Response<Body>> {
    let resp = match (req.method(), req.uri().path()) {
        (&Method::GET, "/") | (&Method::GET, "/index.html") => Ok(Response::new(INDEX.into())),
        (&Method::POST, "/nearby_restaurants") => nearby_restaurants(req, &pool).await,
        _ => {
            // Return 404 not found response.
            Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(NOTFOUND.into())
                .unwrap())
        }
    };

    match &resp {
        Ok(_) => (),
        Err(e) => eprintln!("{:?}", e),
    };
    resp
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    pretty_env_logger::init();
    static MIGRATIONS: Dir = include_dir!("migrations");

    let db_url = env::var("DATABASE_URL")?;
    migrate(&db_url, &MIGRATIONS).await?;
    let pool = PgPool::new(&env::var("DATABASE_URL")?).await?;

    let addr = "127.0.0.1:1337".parse().unwrap();

    // Share a `Client` with all `Service`s
    let client = Client::new();

    let new_service = make_service_fn(move |_| {
        // Move a clone of `client` into the `service_fn`.
        let client = client.clone();
        let pool = pool.clone();
        async {
            Ok::<_, GenericError>(service_fn(move |req| {
                // Clone again to ensure that client outlives this closure.
                router(req, client.to_owned(), pool.clone())
            }))
        }
    });

    let server = Server::bind(&addr).serve(new_service);

    println!("Listening on http://{}", addr);

    server.await?;

    Ok(())
}
