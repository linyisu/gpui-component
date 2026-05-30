# Hello, **World**!

## Task markers

The custom `MarkerHighlighter` (an LSP-style semantic tokens provider)
highlights these markers in the source editor on the left, each in a
different color:

- TODO: support nested task lists
- FIXME: links with parentheses break parsing
- XXX: revisit the table column-width heuristic
- HACK: temporary workaround for footnote ordering
- NOTE: math blocks require the `$$` fence

It also catches markers inside inline `code` and fenced blocks:

```rust
fn render() {
    // TODO: cache the parsed AST between frames
    // FIXME: handle empty input without a panic
}
```

Build Status [![Build Status](https://github.com/longbridge/gpui-component/actions/workflows/ci.yml/badge.svg)](https://github.com/longbridge/gpui-component/actions/workflows/ci.yml) of [GPUI Component](https://github.com/longbridge/gpui-component).

This is first paragraph, there have **BOLD**, _italic_, and ~strikethrough~, `code` text [^1] [^2].

This is an additional demonstration paragraph in English demonstrating more content for [Markdown GFM]. It includes various stylistic elements and plain text.

![Img](https://miro.medium.com/v2/resize:fit:1400/format:webp/1*WgEz5f3n3lD7MfC7NeQGOA.jpeg)

这是一个中文演示段落，用于展示更多的 [Markdown GFM] 内容。您可以在此尝试使用使用**粗体**、*斜体*和`代码`等样式。これは日本語のデモ段落です。Markdown の多言語サポートを示すためのテキストが含まれています。例えば、、**ボールド**、_イタリック_、および`コード`のスタイルなどを試すことができます。

[Markdown GFM]: https://github.github.com/gfm/

[^1]: This is a footnote example.

[^2]: Here is another footnote.

## Basic formatting

### **Bold** text

You can mark some text as bold with **two asterisks**
or **two underscores**.

### **Italic** text

You can mark some text as italic with _asterisks_
or _underscores_.

### **_Bold and italic_**

Three stars gives **_bold and italic_**

### ~~Strikethrough~~

Using `~~two tildes~~` will strikethrough: ~~two tildes~~

## Blockquotes

> Blockquote: More complex nested inline style like **bold: _italic_**.
> This is second paragraph, it includes a block quote.

And this is next blockquote

> Hello, world!

### Nested blockquotes

> First level
>
> > Second level
> > Third level
>
> ```rs
> const FOO: &str = "bar";
> ```

## Code block

#### Rust

```rust
struct Repository {
    /// Name of the repository.
    name: String,
}

fn main() {
    let _ = Repository {
        name: "GPUI Component".to_string(),
    };

    println!("Hello, World!");
}
```

#### Python

```python
class Repository:
    """A repository."""

    def __init__(self, name: str):
        """Initialize the repository.

        Args:
            name: Name of the repository.
        """
        self.name = name
```

---

## Heading for [Links](https://www.google.com)

Here is a link to [Google](https://www.google.com), and another to [Rust](https://www.rust-lang.org).

## Image

![](https://miro.medium.com/v2/resize:fit:1400/format:webp/1*sOTh1aAl32jxKNuGO0TOcA.png)

### SVG

![Rust](https://rust-lang.org/static/images/rust-logo-blk.svg)

## Table

| Header 1 | Centered | Header 3                             | Align Right |
| -------- | :------: | ------------------------------------ | ----------: |
| Cell 0   |  Cell 1  | This is a long cell with line break. |      Cell 3 |
| Row 2    |  Row 2   | Row 2<br>[Link](https://github.com)  |       Row 2 |
| Row 3    | **Bold** | Row 3                                |       Row 3 |

See the way the text is aligned, depending on the position of `':'`

| Syntax    | Description |   Test Text |
| :-------- | :---------: | ----------: |
| Header    |    Title    | Here's this |
| Paragraph |    Text     |    And more |

## Lists

### Bulleted List

- Bullet 1, this is very long and needs to be wrapped to the next line, display should be wrapped to the next line as well.
  Continuation paragraph that should appear below.
- Bullet 2, the second bullet item is also long and needs to be wrapped to the next line.
  - Bullet 2.1
    This is a deepth continuation paragraph.
    - Bullet 2.1.1
      - Bullet 2.1.1.1
    - Bullet 2.1.2

  - Bullet 2.2

- Bullet 3

### Numbered List

1. Numbered item 1
   1. Numbered item 1.1
      1. Numbered item 1.1.1
   1. Numbered item 1.2
2. Numbered item 2
3. Numbered item 3

### To-Do List

- [x] Task 1, a long long text task, this line is very long and needs to be wrapped to the next line, display should be wrapped to the next line as well.
- [ ] Task 2, going to do something if there is a long text that needs to be wrapped to the next line.
- [ ] Task 3

## Heading

Add `##` at the beginning of a line to set as Heading.
You can use up to 6 `#` symbols for the corresponding Heading levels

## Heading 2

This is paragraph of the heading 2.

### Heading 3

This is paragraph of the heading 3.

#### Heading 4

This is paragraph of the heading 4.

##### Heading 5

This is paragraph of the heading 5.

###### Heading 6

This is paragraph of the heading 6.

## HTML

### Paragraph and Text

<div>
    Here is a test in div.
    <p>This is a paragraph inside a div element, have <a href="https://google.com">link</a>, <strong>bold</strong>, <em>italic</em>, and <code>code</code> text.</p>
    <div>
        <p>This is second paragraph.</p>
    </div>
    A text after div.
</div>

### List

<ol>
<li>Numbered item 1</li>
<li>Numbered item 2</li>
</ol>

<ul>
<li>Bullet 1</li>
<li>Bullet 2</li>
</ul>

### Table

<table>
<thead>
<tr>
<td>Head 1</td>
<td>Head 2</td>
</tr>
</thead>
<tbody>
<tr>
<td><strong>Cell</strong> 1</td>
<td>Cell 2</td>
</tr>
<tr>
<td>Cell 3</td>
<td>Cell 4</td>
</tr>
</tbody>
</table>

### Image

<img src="https://miro.medium.com/v2/resize:fit:1400/format:webp/1*QY36p64kSGfBQsIFci8WBw.png" alt="The Best Programming Languages to Learn in 2025" width="100%" />

## Math

### Inline math

Inline formula: $E=mc^2$.

Multiple formulas in one paragraph: $a^2+b^2=c^2$, $e^{i\pi}+1=0$, and $\sin^2x+\cos^2x=1$.

- $F=ma$
- The gradient vector is $\nabla f(x)=(2x_1,2x_2,\dots,2x_n)$.

Escaped delimiters should stay literal: \$x+y\$ and \$\$a+b\$\$.

### Display math

$$E=mc^2$$

$$
a^2+b^2=c^2
$$

### Fractions and radicals

$$
f(x)=\frac{\sqrt{x^2+1}}{x+1}
$$

### Limits

$$
\lim_{x\to0}\frac{\sin x}{x}=1
$$

### Sums and integrals

$$
\int_0^1 x^2\,dx=\frac13
$$

$$
\sum_{i=1}^n i=\frac{n(n+1)}2
$$

### Matrices

$$
A=\begin{bmatrix}
1&2&3\\
4&5&6\\
7&8&9
\end{bmatrix}
$$

### Vectors and delimiters

$$
\vec a\cdot\vec b
=|\vec a||\vec b|\cos\theta
$$

### Cases and piecewise functions

$$
\begin{cases}
x+y=1\\
2x-y=3
\end{cases}
$$

### Aligned equations

$$
\begin{aligned}
(a+b)^2&=a^2+2ab+b^2\\
(a-b)^2&=a^2-2ab+b^2
\end{aligned}
$$

### Equation environment

$$
\begin{equation}
e^{i\pi}+1=0
\end{equation}
$$

### Greek letters and symbols

$$
\alpha+\beta=\gamma
$$

$$
\infty,\ \partial,\ \nabla,\ \aleph
$$

### Color

$$
\textcolor{blue}{E=mc^2}
$$

### Markdown mixed with math

1. Newton's second law: $F=ma$
2. Mass-energy equivalence: $E=mc^2$
3. Euler's identity: $e^{i\pi}+1=0$

> A blockquote with inline math $a^2+b^2=c^2$ and display math:
>
> $$
> \int_0^1 x^2\,dx=\frac13
> $$

| Case | Left aligned formula | Center aligned formula | Right aligned formula |
|---|:---|:---:|---:|
| Short identity | $e^{i\pi}+1=0$ | $e^{i\pi}+1=0$ | $e^{i\pi}+1=0$ |
| Longer expression | $P(A\mid B)=\frac{P(B\mid A)P(A)}{P(B)}$ | $P(A\mid B)=\frac{P(B\mid A)P(A)}{P(B)}$ | $P(A\mid B)=\frac{P(B\mid A)P(A)}{P(B)}$ |

Indented display math in a list:

1. A list item

   $$
   x^2+y^2=z^2
   $$

2. Another list item

   $$
   \int_0^1 x^2\,dx=\frac13
   $$

### Complex formula

$$
\sum_{i=1}^{n}
\left(
\frac{\sqrt{x_i^2+y_i^2}}{1+\int_0^1 t^2dt}
\right)^2
$$

## Unsupported

### HTML

<details>
<summary>Click to expand</summary>
<div>
    <p>This is a paragraph <a href="https://google.com">inside</a> a details element.</p>
    <p>This is second paragraph.</p>
</div>
</details>


This is final paragraph, it includes a code block and a list of items.
