//! DoH 自检：`cargo run -p uurelaybridge-core --example doh_probe -- <url> <domain>`
//!
//! 例：
//!   cargo run -p uurelaybridge-core --example doh_probe -- https://dns.alidns.com/dns-query example.com
//!   cargo run -p uurelaybridge-core --example doh_probe -- http://127.0.0.1:8053/dns-query test.local

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: doh_probe <doh-url> <domain>");
        std::process::exit(2);
    }
    let url = &args[1];
    let domain = &args[2];
    match uurelaybridge_core::sni::dns::resolve_any(domain, url) {
        Ok(ip) => println!("{domain} -> {ip}"),
        Err(e) => {
            eprintln!("解析失败: {e}");
            std::process::exit(1);
        }
    }
}
