# 文献编译诊断

[文档目录](README.md) · [编译诊断](diagnostics.md#bibtex-与-biber) · [文献资源](biblatex-resources.md)

服务器读取本次编译新增或更新的 `.blg` 文件，分析 BibTeX 的重复条目、语法错误和字段警告，以及 Biber 的错误/警告；能识别数据库和文献键时定位到 `.bib` 文件。未能定位的消息显示在主文档。旧 `.blg` 不会被当成本次编译错误。日志中实际读取的 `.bib` 和 `.bst` 也会加入项目依赖，支持宏展开后的文献路径、文献键重命名、根文件识别、未保存检查和过期 PDF 检查；系统目录中的样式不会批量索引。文献记录独立于 `.fls` 保留，直到新的文献日志更新它，避免只运行 TeX 时丢失文献来源。

## 数据库角色与特殊后缀

Biber 的完整 `INFO - Found BibTeX data source '文件名'` 记录确认该文件作为 BibTeX 数据库读取。数据库可以使用 `.data` 等其他扩展名，也可以没有扩展名；即使名称以 `.tex`、`.png` 或 `.bst` 结尾，仍按文献数据索引，不导入其中看似 LaTeX 宏的文字。警告、示例文字、不完整记录或截断日志不会建立新的数据库依赖。

这些来源参与文献导航、版本化文献键重命名、文件移动、未保存检查和编译结果有效性检查。数据库角色随文件移动保留，并以新的完整文献日志更新；数据库与样式仍分别遵循 `BIBINPUTS` / `BSTINPUTS` 的搜索顺序，符号链接使用实际目标。

数据库的完整日志名称也用于警告中的引号路径、裸路径和条目字段定位，包括 Biber 的 `Duplicate entry key` 和 `Datamodel: Entry` 消息。无法确认的显式文件保留在主文档，不借用其他数据库的同名条目。

## 日志记录边界

BibTeX 的数据库、样式和辅助文件记录按完整文件名处理；路径中的 `ERROR -`、`WARN -`、`---line … of file …` 等文字不会生成额外错误。语法错误后的 ` : ` 源码回显也保留为上下文，里面的日志样式文本不会成为新诊断。

Biber 的级别从当前记录头读取，支持普通 `WARN - …`、`[0] WARN - …` 和 `[0] Utils.pm:395> WARN - …` 格式。警告正文中的 `ERROR -` 不改变级别，正文中的 BibTeX 行号样式不改变诊断来源。`INFO` / `DEBUG` 消息里的错误示例不会产生错误；数据库角色只采纳实际 `INFO` 记录中完整的 `Found BibTeX data source '…'` 消息。

## 缺失样式文件

BibTeX 报告无法打开样式文件时，诊断代码为 `missing-bibliography-style`。最终 TeX 日志确认读取的来源中只有一个对应的静态 `bibliographystyle` 声明时，错误定位到精确样式名，并提供最多 5 个同目录本地 `.bst` 拼写建议；保留逗号名称、显式后缀和周围注释。动态或拼接名称、声明歧义、来源不完整及扫描超限时保留主文档中的错误，不提供猜测修复。样式已经恢复或诊断过期时不再修复。

## 缺失文献库

BibTeX 无法打开数据库时，诊断代码为 `missing-bibliography-database`。实际读取的项目源码中存在唯一静态 `bibliography` / `nobibliography` 声明时，精确标出列表中的缺失项，提供最多 5 个同目录本地 `.bib` 拼写建议；保留其他数据库、注释、目录及显式后缀。按 BibTeX 规则匹配名称中的空白，候选不引入空白或逗号。系统宏包实现不充当用户声明，本地宏包、文档类及已索引的外部来源仍参与校验；动态名称、重定义、歧义、来源不完整或日志截断时保留主文档错误，不提供猜测修复。修复兼容版本化编辑和旧客户端，并在应用前复查版本与文件是否已恢复。

Biber 的完整错误 `Cannot find 'reffs.bib'!` 同样使用 `missing-bibliography-database`，支持定位到唯一活动的 `addbibresource` / `addglobalbib` 文件名。BibTeX 后端的缺库错误也支持这两个命令。控制文件缺失、警告、示例文字和不完整消息不会被当作这类缺库错误。

资源参数按完整字面文件名匹配，保留选项、空格、撇号、逗号、`%`、Unicode 和显式扩展名。修复只替换文件参数，并保留完整扩展名；例如 `{reffs.bib}` 可修复为 `{refs.bib}`，`{refs}` 可明确修复为 `{refs.bib}`。查找依赖时仍不自动补 `.bib`，详见[文献资源加载规则](biblatex-resources.md)。

最多提供 5 个同目录候选，显式指定后缀时只比较同后缀文件。远程、通配符、动态资源、重定义、重复声明、来源未确认或日志截断时不猜测修改位置；文件已补回或当前调用含义已改变时不提供旧修复。

## 间接调用与修复校验

LSP 编译结果也会定位静态命名调用和纯别名中的缺失文献库与样式，并提供同样的文件名拼写修复。定位依据本次编译实际读取、且通过输入快照校验的项目来源；宏参数内未执行的命令、未调用的宏体和未加载的别名定义不提供定位依据。修复同时复查已记录的宏作用域与当前项目调用，保留间接调用形式、注释及数据库列表；含义已改变或来源不完整时不提供编辑。

## latexmk 的缺库警告

默认 latexmk 确认数据库缺失并跳过 BibTeX 时，服务器也会产生 `missing-bibliography-database` 警告，提供相同的精确定位和修复。编译继续遵守 latexmk 的策略及现有输入一致性检查，预生成 `.bbl` 的项目仍可生成 PDF。警告只采纳本次最后一次目标日志检查后的完整驱动列表，已有的原生 BibTeX 错误优先；没有输出新列表的空操作编译不会新增缺库警告。其他可执行程序、不同作业日志、不完整或截断的输出不进入这项解析。

## Biber 文献集合

Biber 原生的集合警告使用 `invalid-bibliography-set`：集合条目中已不需要的 `crossref`、集合成员中已不需要的 `entryset`，以及集合缺少 `entryset`。前两类标记原始字段名，数据中的 `reason` 为 `obsolete-field`；后一类标记所属条目键，`reason` 为 `missing-entryset`。诊断保留工具报告的级别与消息，不在没有原生报告时静态判断集合是否合法。

字段名按 Biber 的完整 Unicode 折叠匹配，支持 `CROSSREF`、`croſſref` 等原始拼写和动态值表达式。重复字段、模型名称碰撞、不完整赋值或 source map 生成的字段回退到唯一条目键，并标记 `unresolvedField`。缺少 `entryset` 的报告始终使用条目键，因为 source map 可能已移走原始字段。

这类警告没有数据库名，只有此前输入历史完整、全部来源可读、规范等价且区分大小写的所属键唯一时才定位到数据库。来源不明、键有歧义或缓存超限时保留主文档诊断。保存后重新编译更新位置，修复后清除 push/pull 诊断；不提供猜测的字段删除或插入。

```sh
cargo test --locked --test biber_set_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_sets_
```

## Biber 字段校验

Biber 原生 `Datamodel: Entry 'key' (refs.bib): Field 'field' invalid in data model - ignoring` 和 `Invalid field 'field' for entrytype 'book'` 使用诊断代码 `invalid-bibliography-field`，标记明确条目中的唯一字段名。是否支持字段由实际 Biber 数据模型决定，服务器只采纳工具报告，不静态推测自定义字段是否有效。

`invalid-bibliography-field-value` 标记无效值，支持日期及日期范围结束部分的格式警告、数据类型、上下界、模式匹配，以及 ISBN、ISSN、ISMN 校验消息。完整单一字面值标记内部内容，空字面值覆盖定界符；字符串缩写、拼接和其他完整表达式标记整段值表达式，不执行宏展开。消息和级别均保持原生结果，不自动删除字段或改写值。

带文件名的消息只取明确命名的数据库，支持已确认数据库角色的其他扩展名、完整特殊文件名和符号链接。字段仅来自该条目完整正文中的唯一有效赋值；重复字段、未闭合正文、无效表达式及缺失字段（包括仅由继承提供的字段）保留唯一条目键的位置，不借用父条目或其他数据库。条目键缺失或有歧义时保留文件首行；来源不可读、未确认或超限时保留主文档。日期消息中未转义的文件名与值若无法区分，保留主文档警告。

默认 Biber 未开启数据模型校验时，完整的 `legacy year field 'value' in entry 'key' is not an integer - this will probably not sort properly.`、同模板的 `month` 警告、`ISBN 'value' in entry 'key' is invalid - run biber with '--validate_datamodel' for details.`，以及 `Range field 'pages' in entry 'key' is malformed, falling back to literal` 也使用值诊断代码。这些消息没有文件名，仅在此前数据库读取历史完整、所有候选文件可读且规范化后的键在全部候选中唯一时定位相应字段值；符号链接按实际目标去重，不借用后续输入。范围字段按输入拼写定位，支持默认页码及自定义模型范围字段；消息未指出具体片段，因此标记完整值。历史缺失、语法或未知错误、日志截断、规范化键碰撞或读取超限时保留主文档警告。原始值中的引号、文件名及类似日志的文字不会被另行解释为位置，不自动修改日期、ISBN 或页码范围。

匹配按 Biber 的 Unicode 规则区分命名空间：键保持大小写并规范分解；未知字段、日期及畸形范围警告保留输入字段的组合后 ASCII 折叠；条目类型限制、数据类型及其他数据模型校验、默认年份、月份与 ISBN 警告使用完整 Unicode 折叠。例如自定义模型 `strasse` 可对应原始 `Straße`，默认 `isbn` 可对应 `IſBN`。同条目中的 `STRASSE` 和 `Straße` 对模型警告构成歧义，即使有拼写恰好相同的字段也回退到键；未知字段消息仍能分别定位这两个原始字段，畸形范围警告也能分别定位 `pages` 和 `pageſ` 的值。范围始终指向原始源码，保留 UTF-16 和 CRLF；两种字段索引共享每个原始位置的一次转换，按有界来源建立缓存。保存后重新编译会更新范围，修复后推送与拉取诊断都会清除。

```sh
cargo test --locked --test biber_field_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_field_warnings_
cargo test --locked --test protocol biber_unqualified_field_warnings_
```

## Biber 日期覆盖

完整的 `Overwriting field 'year' with year value from field 'date' for entry 'key'` 和对应的 `month` 警告使用独立代码 `overwritten-bibliography-field`，标记被覆盖的原始年份或月份值。诊断保留工具消息、级别及 `overwrittenBy: "date"` 数据；值本身可以有效，只是与 Biber 实际使用的日期分量不同，不自动决定应删除或修改哪个字段。

条目内存在唯一、完整的原始 `date` 赋值时，标准 `relatedInformation` 关联到它的值，便于从被覆盖值跳转查看日期。范围日期关联整个值，字符串及拼接表达式保持原始表达式范围。原始年份或月份缺失、重复或不完整时回退到唯一条目键；`date` 缺失、歧义或由 source map 生成时不猜测关联位置，不借用其他条目中的字段。

这类消息没有文件名，复用完整的此前 Biber 输入历史、区分大小写的规范化键唯一性和有界来源缓存。来源未确认、键有歧义、日志截断、此前语法或未知错误、读取失败时保留主文档诊断。保存后重新编译更新主位置及关联位置；调整日期或使分量一致后，推送与拉取诊断一并清除。

```sh
cargo test --locked --test biber_overwritten_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_overwritten_fields_
```

## Biber 重复文献键

完整的 `Duplicate entry key: 'alpha' in file 'refs.bib', skipping ...` 消息使用诊断代码 `duplicate-bibliography-key`；开启 `--noskipduplicates` 时没有 `, skipping ...` 的原生格式也受支持。主键仅在明确命名的数据库中区分大小写匹配，唯一声明使用精确键范围；同一文件有多个同名声明时保留文件首行范围，通过 `relatedInformation` 列出候选，不凭无行号消息猜测具体声明。

匹配遵守 Biber 的 Unicode 规范分解，组合形式 `café` 与分解形式 `café` 属于同一键；同库同时存在两种形式时保留歧义并列出原始声明范围，不把报告中的分解形式误当作唯一声明。大小写和兼容字符仍按各自键匹配，不执行兼容分解。规范等价拼写共享 16 项候选上限。

相关位置取当前文件及此前日志确认读取的数据库，最多 16 项，符号链接按实际目标去重；唯一主位置不会再次列为相关位置。未解析来源、不可读或超限文件保留主文档警告；缺失主键不借用其他数据库。日志截断、语法或未知错误、不完整来源历史会停止跨库关联，仍可显示当前明确文件中的候选。沿用大小写警告的完整文件名、数据库角色和有界缓存规则。

诊断保留原生级别与消息。Biber 的重复键选择随选项变化，服务器不把候选位置标为保留或删除项，也不提供自动删除或重命名。保存后重编译会刷新相关位置，修复后清除推送与拉取诊断。同一次日志中按来源缓存键的 UTF-16 范围和首行范围，支持大量条目集中在同一长行的数据库。

```sh
cargo test --locked --test biber_duplicate_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_duplicate_warnings_
```

## Biber 文献键大小写警告

Biber 的完整 `Possible typo (case mismatch) between datasource keys: 'ALPHA' and 'alpha' in file 'refs.bib'` 消息使用诊断代码 `bibliography-key-case-mismatch`，精确标记明确报告的数据库中唯一的第一个键。匹配区分大小写，支持 Unicode、UTF-16、CRLF 和含撇号的完整文件名；其他后缀的文件须已确认数据库角色。

主键与关联键均按 Biber 的 Unicode 规范分解匹配，但范围指向原始源码。例如日志中的 `CAFÉ` / `café` 可以对应源码中的 `CAFÉ` / `café`。规范化后主键仍有多个声明时保留文件首行；另一种拼写的多个规范等价声明保留为独立关联位置。普通 BibTeX 导航及其他后端的名称规则不受影响。

`relatedInformation` 链接另一种拼写在当前数据库及此前已确认读取数据库中的位置，最多 16 处；同名声明有多个时保留多个候选。后续数据库不参与定位。日志截断、此前 Biber 语法或未知错误、来源未解析或读取历史超限时，只使用当前明确命名的数据库；缺失或重复的主键保留文件首行警告，不借用其他数据库。来源不可读、未确认或超出有界解析缓存时保留主文档警告，相关键缺失不影响原始消息。

Biber 将这两个键视为不同条目，并可能保留两者；诊断维持原生警告级别，不提供自动合并或重命名。引用键与数据库键之间的大小写消息仍按原有日志规则处理。修复并重新编译后，推送和拉取诊断同步清除旧结果。

```sh
cargo test --locked --test bibliography_case_diagnostics -- --include-ignored
cargo test --locked --test protocol biber_case_warnings_
```

## 未定义字符串缩写

BibTeX 的两行 `Warning--string name "jouranl" is undefined` / `--line 2 of file refs.bib` 警告使用诊断代码 `undefined-bibliography-string`，精确标记该行中唯一的裸缩写引用。支持字段、`@string`、`@preamble` 和多行 `#` 拼接，按 ASCII 大小写不敏感匹配，并保留 UTF-16 与 CRLF 位置。字面值、声明名、文献键和注释不充当引用；同一行重复引用时保留整行诊断，不猜测修改位置。来源缺失、位置无效或位置行不紧随警告时也保留警告。

快速修复最多提供 5 个相近名称，只取警告发生前已读取数据库中的有效 `@string` 声明，以及当前库中位于引用之前的声明。同名声明使用最近一次的拼写；后置声明、独立项目和隐含月份名不作为候选。月份缩写由样式提供，服务器不解析 `.bst` 宏定义。

修复复查已读取数据库和可定位样式的内容、当前缓冲区、版本及编译输入快照，兼容版本化编辑与旧客户端。日志截断、数据库历史缺失或异常、此前发生语法错误、来源超限或输入变化时保留诊断并停止提供修复。单文件最多读取 2 MiB，累计最多 16 MiB；不完整的 `@string` 赋值不作为修复候选。

验证命令：

```sh
cargo test --locked --test bibliography_string_diagnostics
cargo test --locked --test bibliography_string_diagnostics real_bibtex -- --include-ignored
cargo test --locked --test protocol bibliography_string_warnings_
```

## 未定义条目类型

BibTeX 的两行 `Warning--entry type for "entry" isn't style-file defined` / `--line 2 of file refs.bib` 使用诊断代码 `undefined-bibliography-entry-type`。警告行号指向键名所在行，服务器通过解析索引记录的键和类型关系，标记对应条目的类型名称。例如 `@artcle` 标记 `artcle`，类型与键分行时也能定位。

来源、行号和完整键共同确定条目；同名键出现在多个数据库或不同行时，使用明确报告的数据库与行号。支持 Unicode、CRLF、两种定界符和未完成正文；相同行中有多个匹配键时保留整行警告。来源或位置缺失时保留主文档警告，辅助文件和样式的别名不会因指向相同数据库而提供类型范围。

类型是否受支持由当前文献样式决定，此处只采纳编译器确认的警告。服务器尚未解析样式中的类型定义，因此不生成类型拼写修复；条目类型的编辑补全仍可使用。

```sh
cargo test --locked --test bibliography_type_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_type_warnings_
```

## 无效的 crossref

原生 BibTeX 的两行 `A bad cross reference---entry "child"` / `refers to entry "parent", which doesn't exist` 使用错误代码 `bad-bibliography-crossref`，定位到子条目中唯一的字面 `crossref` 值。字段名不区分 ASCII 大小写，支持花括号、引号、两种条目定界符、UTF-16、CRLF 和数据库符号链接。

字面值前后的 ASCII 空白按 BibTeX 的报告匹配，范围只覆盖父键；空值同样保留完整错误，范围覆盖其花括号或引号，便于在编辑器中看见。

定位只使用报错前日志确认读取的数据库，并要求完整的子条目、唯一键和唯一 `crossref` 字段。动态值、拼接、重复字段、歧义键、缺失数据库、异常读取顺序、此前语法错误、截断日志或扫描超限时，在主文档保留原始错误。缺少第二行时仍保留第一行错误，并继续处理后续独立诊断。沿用单文件 2 MiB、累计 16 MiB 和 1024 个输入的限制，另限制最多扫描 16384 个条目。

已读取数据库中存在同名父条目时，通过 `relatedInformation` 提供最多 16 个相关位置。父条目排在子条目前时，说明 BibTeX 可能跳过未显式引用的前置父条目：父条目确实存在，仍可能报“不存在”。显式引用父条目、使用通配引用和将父条目放在子条目之后会影响 BibTeX 的选择；服务器不把物理存在性当作成功解析的依据，也不自动移动条目或提供未经选择规则验证的拼写修复。

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_crossref_errors_
```

## 嵌套的 crossref

原生 BibTeX 的两行 `Warning--you've nested cross references--entry "child"` / `refers to entry "parent", which also refers to something` 使用诊断代码 `nested-bibliography-crossref`。警告保留完整的子、父条目关系，定位到子条目的字面 `crossref` 值，并在 `relatedInformation` 中链接父条目键。父条目可排在子条目之前或之后；相关说明标明编译器报告的嵌套引用链，不把已经解析到的父条目解释为缺失。

普通嵌套、自引用和循环均按实际编译器日志处理，级别保持为警告，不单凭这些警告将成功构建改成失败。若父条目又引用了缺失祖先，紧随其后的 `bad-bibliography-crossref` 错误独立定位到父条目的字段值，保留错误级别；两条诊断同时显示。

定位复用上述数据库历史、唯一键、唯一字段、完整正文和读取上限。动态或有歧义的子条目引用、缺失来源、截断日志以及缺失或不匹配的续行，都会在主文档保留警告；不会吞掉下一条独立记录。该诊断只采纳原生编译器报告，不静态推测动态继承关系，也不自动改写继承链。修复并重编译后，推送和拉取诊断均清除旧结果。

```sh
cargo test --locked --test bibliography_crossref_diagnostics -- --include-ignored
cargo test --locked --test protocol bibliography_nested_crossrefs_
```

## 重复字段

原生 BibTeX 的 `Warning--I'm ignoring entry's extra "title" field` 及紧随的 `--line … of file …` 使用诊断代码 `repeated-bibliography-field`。诊断精确标记被忽略的字段名，`relatedInformation` 链接同一条目中首次赋值的字段名。保留警告级别、原始键和字段信息，支持 ASCII 大小写变化、含引号或撇号的键、自定义字段名、UTF-16、CRLF、两种条目定界符及数据库符号链接。

BibTeX 的报告行指向解析完字段值后看到的逗号或条目关闭符，可能位于字段名之后数行。定位复用完整值表达式和解析边界，包含 `#` 拼接及后移到下一行的定界符。同一行存在多个重复字段时，只有完整日志中的对应警告数与源码候选数一致，才按源码顺序分别定位；不同字段或报告行独立计数。

同组警告复用字段匹配和位置转换结果，数千个重复字段仍逐一显示精确范围及首次赋值。跨行字面值、Unicode 字段名和 CRLF 均保留原始位置；后续语法错误使读取历史不完整时，后续警告重新核对顺序定位条件。位置缓存仅在本次日志中使用，并与实际解析的来源绑定。

缺失续行或未确认的数据库来源保留主文档警告；行号无效、字段或条目有歧义、赋值无效、日志不完整或无法确认顺序时，保留可确认的报告行。字面文本、注释、特殊构造和其他条目不提供字段位置。该警告按编译器实际忽略的值定位；保存修改并重编译后，push/pull 诊断更新位置，消除重复后清除旧结果。

```sh
cargo test --locked --test bibliography_field_diagnostics -- --include-ignored
cargo test --locked --test protocol repeated_bibliography_fields_
```

## 来源与条目定位

BibTeX 带行号的文献库及样式错误按日志中的输入名称识别类型，再定位到符号链接的实际文件；目标使用无关扩展名时仍保留正确行号。辅助文件不会因链接目标带有源码扩展名而被误当成文献来源；路径缺失或实际为目录时，诊断保留在主文件。

Biber 消息中的文献路径按完整文件名边界匹配，`refs.bib.backup`、`refs.bib2` 或目录名中的 `.bib` 不会借用已有 `refs.bib` 的位置。显式的 `in file refs.bib` 和 `in file 'refs.bib'` 优先于错误正文中的文件名；普通文本 `in parser` 不会阻止识别后面的真实来源。引号内的 `in 'wrong.bib'` 或 `file 'wrong.bib'` 不会覆盖显式来源；无法确认的来源保留原错误信息并在主文件显示。

没有显式来源字段时，独立引号包围的完整文件名、`refs.bib:` 及括号中的路径仍可定位；不会从更大的引号文本中截取内嵌文件名。未加引号的路径保留撇号、逗号和目录中的 `.bib`，兼容消息标点及 Unicode 空白分隔符。

日志明确指出的带引号文献路径会作为完整名称处理，保留空格、撇号和文件名内部的 `.bib` 片段。`refs.bib':copy.bib` 等文件名中紧接引号的标点也保留为名称的一部分，文件名中的 `line 700` 不会覆盖真实行号；这些完整名称也用于依赖索引、跳转和文献键重命名。条目警告只在指定数据库中定位文献键；指定文件不存在、路径为目录或引号无法确认时，在主文件标记来源未确认，不借用前缀、后缀或其他数据库的位置，也不将其他文件的条目范围套到当前文件。

当引号后的标点和空格也属于文件名（如 `refs.bib': copy.bib`）时，使用文献日志确认的完整数据库名称消除歧义。多个已记录名称同时匹配时优先完整的长名称，再按共同的搜索路径规则查找文件；完整文件缺失时在主文件显示原错误，不借用短文件的位置。文件名中的 `in file`、`entry` 和 `line` 不会被当作日志字段，已记录的短名称也不会截断消息中另一个边界明确的长名称。

诊断没有为文献路径添加引号时，也使用日志确认的完整名称保留空格及文件名内部的引号，支持消息中的成对括号。例如 `refs.bib line 700 copy.bib` 不会截成 `refs.bib`，其中的 `line 700` 也不会覆盖真实行号。显式来源和独立路径均遵守这些规则；完整来源缺失时在主文件显示原错误并标记来源未确认。

显式 `file` / `in file` 来源未列入输入日志时，也可定位包含空格的完整 `.bib` 路径。文件名范围内的 `line`、`entry` 文字和消息中其他引号文件名不会提供错误位置；完整文件缺失或名称使用 `.bib.backup` 等后缀时，保留来源未确认状态。普通说明文字（如 `in parser: ...` 或 `in 'parser'`）不会抢在后续可确认的文件字段之前成为诊断来源。

日志与诊断使用不同路径写法时，也按同一输入识别：支持绝对路径、相对路径、`./` 和指向同一数据库的目录链接，保留符号链接后 `..` 的实际含义。匹配遵守工具工作目录及 `BIBINPUTS` 的顺序，使用日志确认读取的目标；其他目录的同名文件不会冒充该来源。来源缺失或发生切换时保留完整消息并标记来源未确认，不借用干扰文件的文献条目范围。诊断明确指定且真实存在的其他文件，即使未列入输入记录，仍可正常定位。

BibTeX 警告末尾的 `in 键名` 按文献键的原始内容查找，保留键名中的引号；即使键名为 `entry.bib`、且磁盘上存在同名文件，也定位到实际数据库中的条目。Biber 的 `entry '键' in file '路径'` 则分别识别文献键和来源路径；键名中的 `.bib` 不会让同名文件覆盖实际数据库，未闭合的键字段也不会被当成来源路径。显式指定的来源文件仍优先使用。

Biber 的 `entry` / `Entry` 字段支持单、双引号包围，完整保留 `O'Connor2026` 等带撇号的文献键，并按 UTF-16 范围定位。文件名、其他引号内容和 `subentry` 等单词中的同名文字不会被当成条目字段；截断、未闭合的显式键不会借用消息末尾其他条目的位置。

Biber 行号只取来源字段和引号内容之外的完整 `line 整数` 字段，`line 700 refs.bib` 等文件名、引号内的错误内容及 `outline` 等单词不会覆盖真实行号；条目警告也保留键名的精确范围。BibTeX 样式生成的警告即使正文包含 `line 700`，仍按末尾的文献键定位；带源文件和行号的 BibTeX 错误继续使用工具的独立行号格式。

文献依赖和日志定位共用搜索规则，识别 BibTeX 的工作目录与 latexmk 报告的数据库/样式搜索路径，避免主目录和输出目录中的同名文件导致定位错误。Biber 日志中的空格、中文及含单引号的数据库名也会保留。
