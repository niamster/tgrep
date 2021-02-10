# tgrep

`tgrep` is a toy grep that searches regexp recursively and applies [.gitignore](https://git-scm.com/docs/gitignore) rules if specified.

## Installation

`cargo install tgrep`

## Usage

```
> tgrep 'my socks' ~
```

### Normal mode
![Normal mode](/img/tgrep-example.png)

### Verbose output
![Verbose mode](/img/tgrep-example-verbose.png)
