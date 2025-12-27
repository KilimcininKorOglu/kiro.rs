mod kiro {
    pub mod machine_id;
    pub mod parser;
    pub mod provider;
    pub mod token_manager;
    pub mod model {
        pub mod credentials;
        pub mod events;
        pub mod token_refresh;
    }
}
mod model {
    pub mod config;
}
fn main() {
    println!("Hello, world!");
}
