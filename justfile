setup:
    lefthook install
    cd frontend && bun install
    cd napi && bun install

build:
    cargo build --workspace
    cd napi && bun run build

build-frontend:
    cd napi && bun run build
    cd frontend && bun run build

run-cli:
    cargo run

run-frontend:
    cd frontend && bun run dev