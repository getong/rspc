use std::{path::PathBuf, time::Duration};

use async_stream::stream;
use axum::{http::request::Parts, routing::get};
use rspc::{Config, Router2};
use serde::Serialize;
use specta::Type;
use specta_typescript::Typescript;
use tokio::time::sleep;
use tower_http::cors::{Any, CorsLayer};

struct Ctx {}

#[derive(Serialize, Type)]
pub struct MyCustomType(String);

#[tokio::main]
async fn main() {
    let inner = rspc::Router::<Ctx>::new().query("hello", |t| t(|_, _: ()| "Hello World!"));

    let router = rspc::Router::<Ctx>::new()
        .config(Config::new().export_ts_bindings(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bindings-legacy.ts"),
        ))
        .merge("nested", inner)
        .query("version", |t| t(|_, _: ()| env!("CARGO_PKG_VERSION")))
        .query("echo", |t| t(|_, v: String| v))
        .query("error", |t| {
            t(|_, _: ()| {
                Err(rspc::Error::new(
                    rspc::ErrorCode::InternalServerError,
                    "Something went wrong".into(),
                )) as Result<String, rspc::Error>
            })
        })
        .query("transformMe", |t| t(|_, _: ()| "Hello, world!".to_string()))
        .mutation("sendMsg", |t| {
            t(|_, v: String| {
                println!("Client said '{}'", v);
                v
            })
        })
        .mutation("anotherOne", |t| t(|_, v: String| Ok(MyCustomType(v))))
        .subscription("pings", |t| {
            t(|_ctx, _args: ()| {
                stream! {
                    println!("Client subscribed to 'pings'");
                    for i in 0..5 {
                        println!("Sending ping {}", i);
                        yield "ping".to_string();
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            })
        })
        // TODO: Results being returned from subscriptions
        // .subscription("errorPings", |t| t(|_ctx, _args: ()| {
        //     stream! {
        //         for i in 0..5 {
        //             yield Ok("ping".to_string());
        //             sleep(Duration::from_secs(1)).await;
        //         }
        //         yield Err(rspc::Error::new(ErrorCode::InternalServerError, "Something went wrong".into()));
        //     }
        // }))
        .build();

    let (routes, types) = Router2::from(router).build().unwrap();

    types
        .export_to(
            Typescript::default(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bindings.ts"),
        )
        .unwrap();
    return; // TODO

    // We disable CORS because this is just an example. DON'T DO THIS IN PRODUCTION!
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any);

    let app = axum::Router::new()
        .route("/", get(|| async { "Hello 'rspc'!" }))
        .nest(
            "/rspc",
            rspc_axum::endpoint2(routes, |parts: Parts| {
                println!("Client requested operation '{}'", parts.uri.path());
                Ctx {}
            }),
        )
        .layer(cors);

    let addr = "[::]:4000".parse::<std::net::SocketAddr>().unwrap(); // This listens on IPv6 and IPv4
    println!("listening on http://{}/rspc/version", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await.unwrap(), app)
        .await
        .unwrap();
}
