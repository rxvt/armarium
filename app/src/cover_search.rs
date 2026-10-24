//! Fetching Cover Candidates from Apple Books over HTTP. Requests run on the
//! main loop; nothing is sent until the user opens the Cover picker.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use armarium_core::cover_search::{self, CoverCandidate};
use gtk::glib;
use soup::prelude::*;

const TIMEOUT_SECONDS: u32 = 10;

/// The app's one HTTP session, with the searches it has made.
struct Client {
    session: soup::Session,
    /// Responses by search term. Apple allows about 20 requests a minute, so
    /// a search is made once per run of the app. Failed searches are not kept.
    searches: RefCell<HashMap<String, glib::Bytes>>,
}

thread_local! {
    static CLIENT: OnceCell<Rc<Client>> = const { OnceCell::new() };
}

fn client() -> Rc<Client> {
    CLIENT.with(|cell| {
        cell.get_or_init(|| {
            let session = soup::Session::new();
            session.set_user_agent(concat!("Armarium/", env!("CARGO_PKG_VERSION")));
            session.set_timeout(TIMEOUT_SECONDS);
            Rc::new(Client {
                session,
                searches: RefCell::default(),
            })
        })
        .clone()
    })
}

/// Searches Apple Books for `term`, putting results titled `title` first.
pub async fn search(term: &str, title: &str) -> Result<Vec<CoverCandidate>, String> {
    let client = client();
    let key = term.trim().to_lowercase();
    let cached = client.searches.borrow().get(&key).cloned();
    let json = match cached {
        Some(json) => json,
        None => get(&client, &cover_search::search_url(term)).await?,
    };
    let found = cover_search::parse_results(&json, title).map_err(|e| e.to_string())?;
    client.searches.borrow_mut().insert(key, json);
    Ok(found)
}

/// Downloads the image at `url`.
pub async fn image(url: &str) -> Result<glib::Bytes, String> {
    get(&client(), url).await
}

async fn get(client: &Client, url: &str) -> Result<glib::Bytes, String> {
    let unreachable = |detail: &str| format!("Couldn’t reach Apple Books: {detail}");
    let message = soup::Message::new("GET", url).map_err(|e| unreachable(&e.to_string()))?;
    let body = client
        .session
        .send_and_read_future(&message, glib::Priority::DEFAULT)
        .await
        .map_err(|e| unreachable(e.message()))?;
    match message.status_code() {
        200 => Ok(body),
        code => Err(unreachable(&format!("HTTP {code}"))),
    }
}
