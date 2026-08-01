use stano_route_macros::get;

#[get(path = "/x")]
async fn handler() -> &'static str {
    "ok"
}

fn main() {}
