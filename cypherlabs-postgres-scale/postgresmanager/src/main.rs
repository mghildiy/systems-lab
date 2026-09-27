mod pb;
mod service;

fn main() {
    let req = pb::StartPrimaryRequest {
        data_dir: "test".to_string(),
        port: 5432,
    };
    println!("{:?}", req);
}