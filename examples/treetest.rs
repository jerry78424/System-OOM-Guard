use oomguard::guard;

fn main() {
    let parent: u32 = std::env::args()
        .nth(1)
        .expect("usage: treetest <pid>")
        .parse()
        .expect("pid must be a number");
    let desc = guard::collect_descendants(parent);
    println!("descendants of {parent}:");
    for (pid, name) in &desc {
        let base = guard::image_base_name(*pid);
        println!(
            "  {pid} {name}  (image_base={})",
            base.as_deref().unwrap_or("<err>")
        );
    }
    println!("total={}", desc.len());
}