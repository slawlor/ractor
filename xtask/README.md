This package is included here to support the automatic reporting of code coverage on 
Github. To view code coverage locally:

Do this once to set it up:
```
rustup component add llvm-tools-preview
cargo install grcov
```

Subsequently, run:
```
cargo xtask coverage --dev
```