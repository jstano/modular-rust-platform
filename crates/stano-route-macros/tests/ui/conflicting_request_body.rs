use stano_route_macros::post;

struct AppJson<T>(T);
struct Foo;
struct Bar;
struct Baz;

#[post(path = "/x", request_body = Bar)]
async fn handler(a: AppJson<Foo>) -> AppJson<Baz> {
    todo!()
}

fn main() {}
