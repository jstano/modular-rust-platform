use stano_route_macros::get;

struct AppPath<T>(T);

#[get(path = "/x/{a}/{b}", responses((status = 200, body = String)))]
async fn handler(a: AppPath<String>, b: AppPath<String>) -> &'static str {
    "ok"
}

fn main() {}
