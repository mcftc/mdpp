#!/usr/bin/env python3
"""Generate the ~1 MB Markdown test file used by the bake-off: python3 gen-sample.py > sample-1mb.md"""
import sys
CHUNK = """# Section {n}: MD++ sample

Lorem ipsum **dolor sit amet**, consectetur *adipiscing* elit. Türkçe karakterler: ğüşiöç ĞÜŞİÖÇ. Visit [the docs](https://example.com/docs/{n}) or see [ref link][r{n}].

## Lists

- First item with `inline code`
- Second item
  - Nested item with a [[wiki link {n}]]
- [ ] Task not done
- [x] Task done

1. Ordered one
2. Ordered two

> A blockquote line that wraps a little bit longer than usual to exercise soft wrapping in the editor widget.

```rust
fn main() {{
    let x = {n};
    println!("hello {{}}", x);
}}
```

| Column A | Column B | Column C |
|----------|:--------:|---------:|
| a{n}     | b        | c        |
| longer cell text | 42 | 3.14 |

![image {n}](./images/pic{n}.png)

[r{n}]: https://example.com/ref/{n}

"""
TARGET = int(sys.argv[1]) if len(sys.argv) > 1 else 1_000_000
out, size, n = [], 0, 0
while size < TARGET:
    s = CHUNK.format(n=n); out.append(s); size += len(s.encode()); n += 1
sys.stdout.write("".join(out))
