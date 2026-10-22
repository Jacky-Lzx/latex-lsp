# 撤销定义与字符串测试

[返回文档目录](README.md) · [保存与恢复](string-test-copies.md) · [副本直接调用](string-test-copy-calls.md)

etoolbox 的撤销定义命令会移除当前控制词的含义。服务器将其目标作为命令名操作数，按加载顺序更新字符串测试及其副本的参数边界。

| 命令 | 静态目标示例 |
| --- | --- |
| `undef` | `\undef\saved` 或 `\undef{\saved}` |
| `gundef` | `\gundef\saved` 或 `\gundef{\saved}` |
| `csundef` | `\csundef{saved}` |
| `csgundef` | `\csgundef{saved}` |

提供参数提示及 `undef` / `gundef` 补全片段。目标仍是普通宏引用；符号可唯一确认时，导航和重命名会识别撤销位置。撤销操作不产生新的宏声明，也不会因目标原来叫 `input`、`label`、`newcommand` 或 `verb` 而执行对应的文件、声明或逐字语法。

## 复制、撤销和重新定义

```tex
\usepackage{etoolbox}
\newbool{draft}
\let\original\ifblank
\undef\ifblank
\newcommand{\ifblank}[1]{#1}
\ifblank{\booltrue{draft}}
\original{\boolfalse{draft}}{Empty}{Tokens}
```

`original` 保留撤销前保存的原生字符串含义。新的 `ifblank` 按自定义宏处理，因而将 boolean `draft` 重命名时，声明与 `booltrue` 会更新，`original` 的 `boolfalse{draft}` 保留为字面数据。

撤销后才制作的副本不再继承已移除的字符串含义。单纯调用已撤销的控制词会产生编译错误；服务器不再为它套用原来的字符串数据参数和参数提示，后续花括号内容按普通代码索引。编译日志仍可将未定义命令错误定位到实际调用。

紧随撤销操作的静态 `input` 仍参与项目分析，因此在其他文件中重新定义目标也会更新后续调用。已识别的自定义宏将控制词作为裸操作数读取时，该控制词不会反过来吞掉之后的代码；此规则无需预先撤销定义，详见[作为宏参数的字符串测试命令](string-test-operands.md)。

## 更新与支持边界

依赖的增量编辑、关闭缓冲区后恢复磁盘内容及磁盘通知会刷新相关含义，主文档版本号保持不变。共享文件在不同根中既可能是字符串数据，也可能是撤销后可执行的代码时，拒绝会修改歧义参数的重命名，其他明确符号仍可编辑。

只推断可确认的静态目标。动态命令名或无法确定含义的撤销接口会保留不确定性，不猜测相关字符串参数的编辑。字符串数据和未调用宏正文内的撤销操作不会在声明处执行。被重定义的撤销命令按已知参数边界处理，不当作确定的删除操作。

此处沿用字符串上下文的静态分析范围，不执行任意宏展开，也不模拟分组结束时的自动恢复；`gundef` / `csgundef` 的全局效果与局部撤销的区别不用于推断未建模的组作用域。

## 验证

```sh
cargo test --locked --test undefined_string_tests
cargo test --locked --test protocol string_undefinitions
cargo test --locked --test undefined_string_tests -- --include-ignored
```

真实回归需要 pdfLaTeX、XeLaTeX、LuaLaTeX 和 etoolbox。四种撤销接口和四种字符串谓词分别验证宏名、boolean 重命名前后的字符串、状态及 PDF 页数；另行检查撤销后调用的错误位置和后续代码执行。
