#[macro_use]
extern crate error_chain;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
extern crate serde_json;
extern crate tokio_core;
extern crate url;
extern crate telegram_bot_raw;

use std::sync::Arc;

use futures::{Future, Stream, Poll};
use futures::future::{result};
use hyper::{Body, Method};
use hyper::client::Client;
use hyper_tls::HttpsConnector;
use tokio_core::reactor::Handle;
use url::Url;

use telegram_bot_raw::{Request, Response, ResponseParameters};

const TELEGRAM_URL: &'static str = "https://api.telegram.org/";

error_chain! {
    foreign_links {
        Url(url::ParseError);
        Hyper(hyper::Error);
        Json(serde_json::Error);
    }

    errors {
        TelegramError {
            description: String,
            parameters: Option<ResponseParameters>
        }
    }
}

pub struct TelegramFuture<T> {
    inner: Box<Future<Item=T, Error=Error>>
}

impl<T> Future for TelegramFuture<T> {
    type Item = T;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.inner.poll()
    }
}

#[derive(Debug, Clone)]
pub struct Bot {
    inner: Arc<BotInner>,
}

#[derive(Debug, Clone)]
struct BotInner {
    base_url: Url,
    client: Client<HttpsConnector>,
}

impl Bot {
    pub fn from_token(handle: &Handle, token: &str) -> Result<Self> {
        let base_url = Url::parse(&format!("{}{}/", TELEGRAM_URL, token))?;

        let connector = HttpsConnector::new(1, handle);
        let config = Client::configure().connector(connector);

        Ok(Bot {
            inner: Arc::new(BotInner {
                base_url: base_url,
                client: config.build(handle),
            }),
        })
    }

    pub fn send<Req>(&self, request: Req) -> TelegramFuture<Req::Response>
        where Req: Request + 'static, <Req as Request>::Response: std::marker::Send + 'static {

        let bot = self.clone();
        let name = request.name();
        let url = futures::lazy(move || {
            result(Url::parse(&format!("{}{}", bot.inner.base_url.as_str(), name)))
        }).map_err(From::from);

        let body = futures::lazy(move || {
            serde_json::to_vec(&request).map(Body::from)
        }).map_err(From::from);

        let bot = self.clone();
        let response = url.join(body).and_then(move |(url, body)| {
            let mut http_request = hyper::client::Request::new(Method::Post, url);
            http_request.set_body(body);

            bot.inner.client.request(http_request).map_err(From::from)
        });

        let bytes = response.and_then(|response| {
            response.body().map_err(From::from)
                .fold(vec![], |mut result, chunk| -> Result<Vec<u8>> {
                    result.extend_from_slice(&chunk);
                    Ok(result)
            })
        });

        let future = bytes.and_then(|bytes| {
            result(serde_json::from_slice(&bytes).map_err(From::from).and_then(|value| {
                match value {
                    Response::Success {result} => Ok(result),
                    Response::Error { description, parameters } => {
                        Err(ErrorKind::TelegramError {
                            description: description,
                            parameters: parameters
                        }.into())
                    },
                }
            }))
        });

        TelegramFuture {
            inner: Box::new(future)
        }
    }
}
