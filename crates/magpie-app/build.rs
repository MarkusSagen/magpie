fn main() {
    // The Slint compiler recurses over the .slint AST. Our large `launcher.slint`
    // overflows the default main-thread stack on Windows (build script dies with
    // STATUS_STACK_OVERFLOW / 0xc00000fd). macOS/Linux have larger default stacks
    // and don't hit it, but running codegen on an explicit big-stack thread makes
    // the build robust on every platform.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| slint_build::compile("ui/launcher.slint").unwrap())
        .expect("spawn slint build thread")
        .join()
        .expect("slint build thread panicked");
}
