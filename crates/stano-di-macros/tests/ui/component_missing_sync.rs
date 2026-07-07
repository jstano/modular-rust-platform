use stano_di_macros::component;

#[component]
trait Greeter: Send {
    fn greet(&self) -> String;
}

fn main() {}
