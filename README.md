# tgrep

`tgrep` is a toy grep that searches regexp recursively and applies [.gitignore](https://git-scm.com/docs/gitignore) rules if specified.

## Installation

Install [cargo](https://doc.rust-lang.org/cargo/getting-started/installation.html) and run `cargo install tgrep`.

## Usage

```
> tgrep 'my socks' ~
```

### Normal mode
![Normal mode](https://raw.githubusercontent.com/niamster/tgrep/main/img/tgrep-example.png)

### Verbose output
![Verbose mode](https://raw.githubusercontent.com/niamster/tgrep/main/img/tgrep-example-verbose.png)
