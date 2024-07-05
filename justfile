setup:
    lefthook install
    cd frontend && bun install

run *args="":
    cargo run -p spackle-cli {{args}}
 
install:
    cargo install --path=cli