use oomguard::guard;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid: u32 = args
        .get(1)
        .expect("usage: killtest <pid> [tree]")
        .parse()
        .expect("pid must be a number");
    let tree = args
        .get(2)
        .map(|s| s.eq_ignore_ascii_case("tree"))
        .unwrap_or(false);
    let protected: Vec<String> = args
        .get(3)
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().trim_end_matches(".exe").to_lowercase())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    match guard::enable_debug_privilege() {
        Ok(()) => println!("SeDebugPrivilege: enabled"),
        Err(e) => println!("SeDebugPrivilege: FAILED err={e}"),
    }
    let opts = guard::KillOpts { tree, protected };
    let r = guard::terminate_with(pid, &opts, |msg| println!("{msg}"));
    println!("result={:?}", r);
}
