# text-ref

**在不重复存储的前提下复制、编辑与共享文本。**

[English](README.md) | **简体中文**

`text-ref` 是一个零依赖的小型 Rust 库，围绕两个类型构建：

- `TextRef<'a>` —— 不可变文本快照，克隆开销极低。它要么借用一段 `&str`，
  要么通过 `Arc<String>` 与其他快照共同持有同一份缓冲区。
- `TextMut` —— 独占、可变的对应类型（对 `String` 的轻量包装），用于构建
  文本和执行编辑。

文本可以不复制字节地在各处传递、克隆；可变缓冲区可以零拷贝地“冻结”成共享
快照；需要修改时再显式地按写时复制（copy-on-write）规则编辑：只有当仍然有其
他快照持有这份文本时才会真正复制内容。

## 为什么需要新的字符串类型？

标准库提供了三种选择，每种都有取舍：

- `String` —— 易于编辑，但每次 `clone()` 都会分配内存并复制全部文本。
- `&str` —— 零开销，但受借用生命周期限制，无法独立保存。
- `Arc<str>` —— 克隆和共享都很便宜，但无法在不重新分配的情况下编辑。

`text-ref` 把它们结合起来。快照要么是借用，要么是 `Arc` 共享，因此克隆是
O(1)；真正需要编辑时，再把快照转回独占的 `TextMut`：如果自己是最后一个持有
者，就直接复用原有的内存分配。

## 特性

- **零依赖。**
- **O(1) 快照克隆。** 克隆 `TextRef` 只复制一个指针或增加一次引用计数，绝不
  复制文本。
- **零拷贝冻结。** `TextMut::freeze()` 把 `String` 缓冲区移入 `Arc`，字节
  不会被复制。
- **写时复制编辑。** 共享快照是唯一持有者时，`TextRef::edit()` 直接复用缓冲
  区；仅在仍有其他快照存活时才克隆文本。
- **就地编辑。** `TextRef::edit_in_place()` 通过闭包拿到 `&mut String`，编辑
  后把结果放回原处，并返回闭包的返回值。
- **符合人体工程学。** `TextMut` 解引用到 `String`（`Deref`/`DerefMut`）；
  `TextRef` 解引用到 `str`，并实现 `AsRef<str>`、`Display`、`Clone` 和
  `Debug`。
- **文档完整。** 所有公开项都有 rustdoc，示例会作为文档测试编译并运行。

## 安装

本库尚未发布到 crates.io。可以通过 Git 或本地路径引入：

```toml
[dependencies]
# 来自 Git
text-ref = { git = "https://github.com/afpro/text-ref" }

# 或使用本地检出
# text-ref = { path = "../text-ref" }
```

需要 Rust 1.85 或更高版本（使用 2024 edition）。

## 快速开始

```rust
use text_ref::{TextMut, TextRef};

fn main() {
    // 1. 在独占期间构建文本。
    let mut text = TextMut::with_capacity(64);
    text.push_str("hello");
    text.push_str(", world");

    // 2. 冻结。缓冲区被移入共享存储，不复制任何字节。
    let snapshot: TextRef<'static> = text.freeze();

    // 3. 快照克隆开销极低 —— 所有克隆都指向同一份缓冲区。
    let shown = snapshot.clone();
    let kept = snapshot.clone();
    println!("{shown}"); // TextRef 实现了 Display

    // 4. 编辑采用写时复制；kept 和原始快照看到的仍是旧文本。
    let mut edited = shown.edit();
    edited.push('!');
    assert_eq!(edited.as_str(), "hello, world!");
    assert_eq!(&*kept, "hello, world");
}
```

就地编辑，并让闭包返回一个计算结果：

```rust
use text_ref::{TextMut, TextRef};

fn main() {
    let mut snapshot: TextRef<'static> = TextMut::from("hello").freeze();

    let length = snapshot.edit_in_place(|s| {
        *s = s.to_uppercase();
        s.len() // 返回给调用者
    });

    assert_eq!(length, 5);
    assert_eq!(&*snapshot, "HELLO");
}
```

## 工作原理

```text
  TextMut (独占, 可变)  ── freeze() ──►  TextRef<'static> (共享, 不可变)
        ▲                                              │
        └────────── edit() / edit_in_place() ──────────┘
```

`TextRef` 有两种变体：

| 类型 / 变体 | 底层存储 | 克隆开销 | 可变性 |
| --- | --- | --- | --- |
| `TextMut` | 独占的 `String` | O(n)，复制文本 | 可，通过 `&mut` |
| `TextRef::Ref` | 借用的 `&str` | O(1)，复制胖指针 | 不可 |
| `TextRef::Shared` | `Arc<String>` | O(1)，引用计数加一 | 不可 |

共享变体使用 `Arc<String>` 而不是 `Arc<str>` 是有意为之：`String` 实现了 `Sized`
（`str` 没有），所以 `Arc::try_unwrap` 可以把缓冲区取回来就地编辑；而
`Arc<str>` 想修改就必然要重新分配。

### 写时复制的细节

`TextRef::edit()` 会消耗快照并产生一个 `TextMut`：

- `TextRef::Ref` —— 把借用的字节复制进新的 `String`。
- `TextRef::Shared` —— 尝试 `Arc::try_unwrap`：
  - 自己是最后一个持有者：原样取回 `String`，不复制、不分配；
  - 还有其他快照存活：复制一次文本，这些快照继续指向未改变的原文。

因为快照不可变，共享它们不需要锁或同步：读取方只能看到自己拿到的那份文本。

### `edit_in_place` 的 panic 安全性

`edit_in_place` 先把文本从 `self` 中移出（原位置留下空字符串），编辑临时缓冲
区，再冻结并放回 `self`。如果闭包发生 panic，`self` 会留下空快照
`TextRef::Ref("")`，原内容在栈展开时被丢弃。

## API 一览

| 项 | 说明 |
| --- | --- |
| `TextMut::new()` / `Default` | 空文本，不分配内存。 |
| `TextMut::with_capacity(n)` | 空文本，预分配至少 `n` 字节。 |
| `TextMut::from(value)` | 从任何可转换为 `String` 的值构建（`&str`、`String` 等）。 |
| `TextMut::freeze()` | 把缓冲区无拷贝地移入 `TextRef<'static>`。 |
| `TextRef::Ref(&str)` | 直接构造借用快照。 |
| `TextRef::Shared(Arc<String>)` | 直接构造共享快照。 |
| `TextRef::edit()` | 转换为 `TextMut`；仅在文本被共享时复制。 |
| `TextRef::edit_in_place(f)` | 通过 `&mut String` 编辑，并返回闭包的结果。 |

Trait 实现：

| Trait | `TextMut` | `TextRef<'a>` |
| --- | --- | --- |
| `Clone` | 深拷贝，O(n) | 廉价拷贝，O(1) |
| `Debug` | 是 | 是 |
| `Deref` | → `String` | → `str` |
| `DerefMut` | → `String` | — |
| `AsRef<str>` | — | 是 |
| `Display` | — | 是 |

`TextRef` 没有实现比较 trait；请按字符串切片比较，例如
`&*snapshot == "hello"` 或 `snapshot.as_ref() == "hello"`。

## 性能

| 操作 | 复制的文本 | 内存分配 |
| --- | --- | --- |
| `TextMut::freeze()` | 无 | 一个小的 `Arc` 头 |
| `TextRef::clone()` | 无 | 无 |
| `TextRef::edit()`，唯一持有者 | 无 | 无 |
| `TextRef::edit()`，被共享 | 全部（O(n)） | 一个 `String` |
| `edit_in_place()`，唯一持有者 | 无 | 无 |
| `edit_in_place()`，被共享 | 全部（O(n)） | 一个 `String` |
| `TextMut::clone()` | 全部（O(n)） | 一个 `String` |

推荐的用法是：用 `TextMut` 构建，冻结一次，广泛共享；只有确实需要修改时才
解冻。

## 与其他类型比较

| 能力 | `String` | `Cow<'a, str>` | `Arc<str>` | `TextRef` |
| --- | --- | --- | --- | --- |
| 廉价克隆 | 否 | 仅借用变体 | 是 | 是 |
| 共享同一份缓冲区 | 否 | 否 | 是 | 是 |
| 可持有借用文本 | 否 | 是 | 否 | 是 |
| 共享后编辑无需复制 | — | — | 否 | 是，需为唯一持有者 |

## 线程安全

`TextRef<'static>` 实现了 `Send + Sync`：克隆可以移动到其他线程并发读取，
因为底层文本不可变。`TextMut` 同样是 `Send + Sync`，但修改它需要 `&mut`，
所以跨线程编辑需要外部同步（或每线程各持一份副本）。`TextRef<'a>` 可以借用
非 `'static` 数据；它同样实现了 `Send + Sync`，但不能超出该借用的生命周期，
因此只能在借用有效期内传给其他线程（例如配合 scoped thread 使用）。

## 不适用场景

- 它不是 rope 或文本编辑器：文本被共享时，每次编辑都会复制整个字符串。
- 没有内部可变性：共享快照无法被其他线程就地修改。
- 如果只是构建一次然后只读，直接使用 `String` 或 `&str` 更简单；本库的优势
  在于文本既需要共享、又偶尔需要编辑的场景。

## 开发

```sh
cargo test          # 运行文档测试（包含示例）
cargo clippy
cargo doc --open
```

## 许可证

MIT，见 [LICENSE](LICENSE)。
