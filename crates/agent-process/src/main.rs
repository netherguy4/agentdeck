fn main() {
    std::process::exit(agent_process::main(std::env::args_os().skip(1).collect()));
}
