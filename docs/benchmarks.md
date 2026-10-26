# 连续编辑性能测试

[返回文档](README.md)

`benchmarks/editing.py` 使用 Python 3 标准库启动真实的 release 服务器，以 LSP stdio 协议回放编辑器会话。无需安装 Python 包、TeX 发行版或编辑器。

```sh
cargo build --release --locked
python3 benchmarks/editing.py --output target/benchmarks/latest.json
```

默认测量三档确定性生成的项目：small 为 1 个章节、每章 16 节；medium 为 4 个章节、每章 32 节；large 为 8 个章节、每章 64 节。每个项目包含主文档、章节文件与 BibTeX 数据库，正文含中文、emoji、数学环境、用户宏、跨文件标签和文献引用。项目均低于默认索引预算；测试会拒绝索引截断。

每档启动 3 个独立服务器，每次先预热 3 个周期，再测量 20 个周期。每周期执行：

1. 插入／删除中文，或粘贴／删除带新标签的多行正文，使用 UTF-16 增量范围。
2. 修改未保存主文档中用户宏的参数个数，并在章节中查询最新参数提示。
3. 查询标签、文献、命令补全，悬停，定义，引用，跨文件重命名，参数提示，大纲，工作区符号的精确／全部／无匹配搜索，折叠，以及完整语义令牌和缓存 delta。
4. 引入／修复未定义引用，查询完整 pull 诊断，再用 `previousResultId` 查询缓存报告。
5. 连发 8 次增量编辑，随后立即请求补全，测量输入突发下的响应时间。

重命名只测量生成工作区编辑的时间，不应用重命名。每个响应都会进行内容校验，包括预期补全候选、精确的定义位置、跨章节引用和重命名的完整文件集合及条目数量、主文档参数变化，以及错误出现／消失和诊断缓存命中。CI 运行 small 的短会话校验协议和脚本，不设置受机器负载影响的耗时门槛。

`--hook-labels` 为每节增加显式 `contribution` 代码标签，并加入另一个 hook 的干扰标签和带完整后缀的补全字段。每周期额外核验全部代码块的精确定义位置、注册与使用的全部引用、悬停及按 hook 过滤的完整后缀补全；三档分别有 17／129／513 个定义，18／133／521 项引用。可与其他 hook 选项组合。配对脚本的 `--query hook-definition` 自动启用此负载，交替测量同一代码标签查询，并逐项比较定义响应；`--query definition --hook-labels` 用于观察同一项目内普通查询的索引成本。

```sh
python3 benchmarks/editing.py --hook-labels --output target/benchmarks/hook-labels.json
python3 benchmarks/rename_pairs.py --before target/benchmarks/before --after target/benchmarks/after \
  --query hook-definition --output target/benchmarks/hook-label-pairs.json
```

`--lifecycle-labels` 隐含 `--hook-labels`，把每节的注册改为 `AtBeginDocument[contribution]`，在主文档混合一个 `AddToHook{begindocument}[contribution]`，并加入结束阶段的 `AtEndDocument[contrast]` 干扰。定义／引用按 `begindocument` 组合身份独立计算，悬停也核对 hook 名称；补全必须排除结束阶段和其他 hook 的同前缀标签。仍逐项核验三档全部 17／129／513 个代码块及 18／133／521 项引用。两种脚本均记录 `lifecycle_labels`，比较要求选项一致。

```sh
python3 benchmarks/editing.py --lifecycle-labels --output target/benchmarks/lifecycle-labels.json
python3 benchmarks/rename_pairs.py --before target/benchmarks/before --after target/benchmarks/after \
  --query hook-definition --lifecycle-labels --output target/benchmarks/lifecycle-pairs.json
```

`--global-hook-rules` 隐含 `--hook-labels`，每节加入其他 hook 的同名注册，每章增加全局默认规则及完整后缀补全探测。可与 `--lifecycle-labels` 组合；完整会话同时核验局部和全局定义／引用、悬停作用域及精确补全。全局补全必须包含跨 hook 的 `contrast`、`contribution`，局部补全仍只包含自身 hook 的注册。三档全局定义为 33／257／1025 处，全局引用为 35／265／1041 处；局部定义保持 17／129／513 处，局部引用增加到 19／137／529 处。配对脚本的 `--query global-hook-definition` 自动启用此负载并逐次校验全部全局定义，`--query hook-definition` 继续仅核验当前 hook 的定义，`--query hook-name-definition` 则核验独立 hook 名称的声明，作为不经过代码标签索引的对照。

```sh
python3 benchmarks/editing.py --global-hook-rules --lifecycle-labels \
  --output target/benchmarks/global-hook-rules.json
python3 benchmarks/rename_pairs.py --before target/benchmarks/before --after target/benchmarks/after \
  --query global-hook-definition --lifecycle-labels --output target/benchmarks/global-hook-pairs.json
```

`--default-hook-labels` 隐含 `--hook-labels`，每节增加 Push／Set／省略标签注册／Pop，并在章末增加默认标签设置的定义、引用、悬停和完整后缀补全查询。定义只计当前已认证的显式注册，省略注册不计入预期结果。设置字段的定义与补全跨 hook，局部定义仍按自身 hook 过滤；两类引用均包含相应设置字段。与 `--global-hook-rules --lifecycle-labels` 组合时，三档局部定义／引用为 17／36、129／269、513／1049，全局及默认设置为 33／52、257／397、1025／1561。配对脚本的 `--query default-hook-definition` 自动启用新负载并逐次校验全部显式定义；其他选项仍可独立组合。

```sh
python3 benchmarks/editing.py --default-hook-labels --global-hook-rules --lifecycle-labels \
  --output target/benchmarks/default-hook-labels.json
python3 benchmarks/rename_pairs.py --before target/benchmarks/before --after target/benchmarks/after \
  --query default-hook-definition --global-hook-rules --lifecycle-labels \
  --output target/benchmarks/default-hook-pairs.json
```

## 指标含义

- `initialize`：初始化请求到响应，单独报告，不混入预热后的功能样本；`open` 包含打开两个缓冲区到有序屏障响应。
- `edit.*`：发送单次编辑通知，到后续 `latex.indexingStatus` 请求返回。这个轻量、有序请求是通知处理屏障；耗时包括文本更新、解析、依赖刷新、传输和屏障本身，并非纯解析时间。
- 其他功能：已完成前序编辑后，单次请求到响应的墙钟耗时，包括 stdio、JSON 编解码、排队和功能计算；按会话顺序测量，前面的查询可能预热后面的查询。
- `semantic.full` 在文本修改后查询；`semantic.delta.cached` 在无后续修改时请求 delta。两者明确区分。
- `diagnostic` 在新的引用编辑后重新计算；`diagnostic.cached` 必须返回 `unchanged`。
- `burst8.to_completion`：从第一条编辑通知到补全返回，包含 8 次编辑的处理时间；这是积压场景下的端到端延迟。

输出 JSON 保存所有原始毫秒样本、p50、p95（nearest rank）与最大值，以及项目字节数、项目和脚本指纹、二进制指纹、平台和 Python 版本。各进程的预热后样本合并统计；初始化和打开每个进程各一个样本。

本测试关闭发行版发现，并使用 pull 诊断，避免机器上的 TeX 安装和 push 诊断防抖影响功能比较。它测量语言功能；PDF 编译、外部格式化器、ChkTeX 和磁盘保存不是本次负载。这里使用可复现的合成多文件论文，不代表所有真实用户项目，尤其不覆盖复杂宏展开、超大单行和大量条件加载。

`--hooks` 在主文档加入一个带参数的内核 hook，并在正文使用它。每周期额外测量 `signature.hook`，核验声明的 1 个参数覆盖调用处写出的 0；其他引用、重命名及编辑校验保持完整。章节没有 hook 调用，用于观察项目级参数分析是否给无关章节增加重复扫描。输出参数记录 `hooks`，前后比较必须使用相同选项。

```sh
python3 benchmarks/editing.py --hooks --output target/benchmarks/hooks.json
```

`--hook-wrappers` 包含上述 hook 负载，并在主文档定义两层带参数包装宏，每节增加一次包装调用。它测量把调用实参传递为 hook 字面数据时的完整分析成本；独立标签的跨章节引用和改名仍进行数量及文件集合校验。输出记录 `hook_wrappers`，比较时也必须一致。

```sh
python3 benchmarks/editing.py --hook-wrappers --output target/benchmarks/hook-wrappers.json
```

`--hook-environments` 包含 hook 负载，并为每节加入一次 xparse 环境调用，环境结束正文将开始参数传给 hook。每档分别定义 16／32／64 个环境，各章节复用这些定义，用于测量环境参数绑定随定义数和调用数增长的成本。可与 `--hook-wrappers` 组合；输出记录 `hook_environments`，前后比较要求选项一致。

```sh
python3 benchmarks/editing.py --hook-wrappers --hook-environments \
  --output target/benchmarks/hook-environments.json
```

`--hook-renames` 包含 hook 负载，并增加 `rename.hook`。它从主文件的显式声明请求改名，逐项比较所有源码中完整的名称编辑范围和替换文本，覆盖注册、使用、包装正文和章节转发调用，排除普通缺失引用。配对探测使用 `--query hook-rename`，同样检查完整编辑集合。

`--hook-interfaces` 包含 hook 负载，在主文件定义零参数环境转发器 `benchinterface`，每节直接调用其生成的开始控制序列，把紧随的名称与字面数据交给 `UseHook`。它覆盖环境生成接口的安全分析成本，可与上述两种包装负载组合；输出记录 `hook_interfaces`。

```sh
python3 benchmarks/editing.py --hook-wrappers --hook-environments --hook-interfaces \
  --output target/benchmarks/hook-interfaces.json
```

## 比较优化前后

先保留优化前的二进制，再构建优化后的版本，依次测量，避免构建或其他 benchmark 同时运行：

```sh
cargo build --release --locked
mkdir -p target/benchmarks
cp target/release/latex-lsp target/benchmarks/latex-lsp-before
# 修改源码后重新构建。
cargo build --release --locked
python3 benchmarks/editing.py --server target/benchmarks/latex-lsp-before \
  --output target/benchmarks/before.json
python3 benchmarks/editing.py --baseline target/benchmarks/before.json \
  --output target/benchmarks/after.json
```

比较要求脚本指纹、预热次数、采样次数、运行次数及项目指纹一致；使用同一机器、相同构建参数，并保持负载稳定。速度倍率是优化前 p50 / 优化后 p50；小于 1 表示变慢。不要把一次运行的亚毫秒差异当作回归证据。

生成可切换项目规模和 p50／p95 的独立 HTML 对比页：

```sh
python3 benchmarks/report.py target/benchmarks/before.json target/benchmarks/after.json \
  --output target/benchmarks/report.html
```

页面自包含，可离线打开或分享；完整原始样本保留在 JSON 中。

快速检查可以使用：

```sh
python3 benchmarks/editing.py --scales small --iterations 2 --warmup 1 \
  --runs 1 --output target/benchmarks/smoke.json
```

`--timeout` 设置每个请求等待上限，默认 120 秒；服务退出、超时、协议错误和功能校验失败都会让脚本非零退出。

## 2026-10-26 第四轮：hook 项目的重复扫描

补齐静态可变参数 hook 后，分别测量常规项目与 `--hooks` 项目。每档 3 个独立进程，每个进程预热 3 个周期、测量 12 个周期，每项 36 个样本；构建、测试和测量分别运行。

审阅发现，改名保护会先为每份章节构造完整引用上下文，再确认该文件是否包含相关调用；参数扫描也会为普通命令反复探测四个 hook 接口。现在先根据可能的直接／命名／转发／复制名称筛选章节，并直接检查已经解析的叶命令。分析预算、数据边界与歧义保护保持原有规则。

以下为 large 项目的端到端毫秒。复测交换两版运行顺序，单独保存，不与首轮样本混合；两组速度倍率不是置信区间。

| 负载与指标 | 优化前 p50 | 优化后 p50 | 速度倍率 | 优化前 p95 | 优化后 p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 带 hook：跨文件重命名 | 24.89 | 21.68 | 1.15× | 41.25 | 39.18 |
| 带 hook：交换顺序复测重命名 | 26.37 | 20.85 | 1.27× | 41.82 | 37.32 |
| 带 hook：标签补全 | 30.93 | 28.43 | 1.09× | 48.42 | 52.94 |

主要可重复收益是带 hook 大型项目的重命名。标签补全的 p50 略降，但 p95 首轮变差；常规项目和其他功能没有一致的广泛提升，不据此宣称整体加速。新增 `signature.hook` 同时校验真实数量覆盖，不仅记录耗时。

原始样本：[常规优化前](../benchmarks/results/2026-10-26/round4-before.json)、[常规优化后](../benchmarks/results/2026-10-26/round4-after.json)、[hook 优化前](../benchmarks/results/2026-10-26/round4-hooks-before.json)、[hook 优化后](../benchmarks/results/2026-10-26/round4-hooks-after.json)、[交换顺序复测前版](../benchmarks/results/2026-10-26/round4-hooks-repeat-before.json)、[交换顺序复测后版](../benchmarks/results/2026-10-26/round4-hooks-repeat-after.json)。传给 `benchmarks/report.py` 可以重建独立 HTML 对比页。结果仅适用于本机合成论文负载。

## 优化内容

导航、引用、重命名、悬停和参数提示现在使用与补全和诊断相同的查询内源码图缓存。同一查询还复用最多 16 组依赖遍历结果，键包含工作区、起始文档和根文档，避免重复文件解析与路径检查；查询结束即释放，后续编辑或磁盘变化重新计算。

标签声明的非执行数据分析原先会对每一个消费文档，重新分析项目中每份源码的命令执行位置。没有宏声明的章节不会贡献任何数据读取器，现在直接跳过这次扫描，减少多章节项目里的重复工作。声明、引用及未保存内容的语义保持原有规则。

命名引用分析还会被同一请求中的目标识别、作用域校验和引用收集重复使用。引用、重命名及其预检查通过不可变共享索引复用这些结果；这些查询和诊断也按精确项目范围共享标签声明分析。补全、定义、悬停和参数提示仅使用源码图缓存，避免为只使用一次的分析保留额外索引。两类符号分析各最多保留 4 项，共享每次查询约 8 MiB 的估算字节预算；超限仍正常计算结果，不额外缓存。缓存键区分工作区和原始查询文档／项目范围，单独提供资源上下文的文献分区分析继续使用自己的解释，避免混用含义。

## 2026-10-26 第一轮测量结果

环境：Linux x86_64，Intel Xeon Gold 6146，默认 release 配置（thin LTO）。每档 3 个独立进程，每个进程预热 3 个周期、测量 12 个周期；每项功能 36 个样本。两版在同一机器上顺序运行，测量期间没有并行构建或测试。

large 项目包含 10 个文件、196343 字节；以下为端到端 p50，单位毫秒：

| 功能 | 优化前 | 优化后 | 速度倍率 |
| --- | ---: | ---: | ---: |
| 标签补全 | 48.25 | 22.11 | 2.18× |
| 悬停 | 67.17 | 15.43 | 4.35× |
| 定义跳转 | 91.91 | 25.13 | 3.66× |
| 查找引用 | 118.07 | 36.54 | 3.23× |
| 跨文件重命名 | 143.10 | 53.43 | 2.68× |
| 重新计算诊断 | 111.47 | 37.37 | 2.98× |
| 连续 8 次编辑到补全 | 61.76 | 34.18 | 1.81× |

优化集中于依赖和标签分析。单次文本编辑、折叠、语义令牌及缓存诊断的中位数基本保持原水平；这些指标中的微小变化不足以说明性能差异。

保留了三档项目的完整指标、环境和原始样本：[优化前 JSON](../benchmarks/results/2026-10-26/before.json)、[优化后 JSON](../benchmarks/results/2026-10-26/after.json)。可将它们传给 `benchmarks/report.py` 重建 HTML 对比页。这是单机合成论文负载的测量，不能外推为所有项目的固定加速倍数。

## 2026-10-26 第二轮测量结果

本轮基线为第一轮优化后的版本，优化后为新增引用／标签分析复用的最终版本。环境、构建配置及采样参数与上表相同；两版重新顺序测量，期间没有并行构建或测试。large 项目的端到端延迟如下，单位毫秒：

| 功能 | 基线 p50 | 本轮 p50 | p50 速度倍率 | 基线 p95 | 本轮 p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 查找引用 | 36.34 | 22.49 | 1.62× | 44.35 | 29.44 |
| 跨文件重命名 | 53.71 | 18.55 | 2.90× | 92.41 | 33.60 |
| 重新计算诊断 | 37.72 | 30.23 | 1.25× | 61.25 | 51.24 |

small／medium 的重命名 p50 分别从 2.62／14.20 ms 降到 1.17／5.04 ms。large 的标签补全为 22.40 → 22.57 ms，连续 8 次编辑到补全为 35.02 → 35.28 ms，正文编辑处理为 2.06 → 2.07 ms；本轮没有明显改善这些指标。p95 更容易受负载影响，完整数据保留了其他指标的波动，不把它们解读为全面加速。

新增符号缓存只用于有重复分析的查询。扩大到所有查询的实验曾让补全等路径的中位数增加约 10%，反转测量顺序后仍然存在；缩小缓存范围后重新测量得到上面的最终结果。

本轮还增强了定义精确范围、跨文件引用完整数量和重命名编辑的校验，因此脚本指纹与第一轮不同。请在同一轮内配对报告，不能将两轮的 JSON 交叉比较。最终原始样本：[本轮基线 JSON](../benchmarks/results/2026-10-26/round2-before.json)、[本轮优化后 JSON](../benchmarks/results/2026-10-26/round2-after.json)。

```sh
python3 benchmarks/report.py benchmarks/results/2026-10-26/round2-before.json \
  benchmarks/results/2026-10-26/round2-after.json \
  --output target/benchmarks/round2-report.html
```

最终代码通过完整测试：3341 项通过，490 项需要外部工具的测试跳过；另通过格式检查、严格 Clippy，以及 CI 参数的真实 stdio 短会话。回归覆盖查询后的未保存依赖／宏含义变化、工作区和项目范围隔离、嵌套查询异常恢复，以及缓存容量超限时继续计算。


## 2026-10-26 第三轮：工作区符号搜索

基线为 `cca0e35` 的 release 版本。本轮新增工作区符号的精确、全部和无匹配查询，保留原有文档大纲、编辑、补全、诊断等指标作为对照。三档项目各启动 3 个独立进程，每个进程预热 3 个周期，测量 8 个周期，每项 24 个样本；脚本和项目指纹一致。最终配对先测优化版本，再测基线版本，测量期间没有构建、测试或其他 benchmark 同时运行。large 项目仍为 10 个文件、196343 字节。

初轮分析确认，每次搜索都会重建所有文档的大纲，分配完整平面结果，再排序截取前 256 项。现在按源码版本共享有界大纲缓存，并用最多 256 项的排名队列保留候选，避免复制所有匹配项和同时保留超限文档的大纲。缓存不保存 URI，总保留预算约 4 MiB；编辑、语言角色改变和移动时重新生成，超限继续计算。详见[索引与缓存](indexing.md)。

large 的端到端延迟，单位毫秒：

| 功能 | 基线 p50 | 本轮 p50 | p50 速度倍率 | 基线 p95 | 本轮 p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 精确搜索 | 1.996 | 0.585 | 3.41× | 3.289 | 0.947 |
| 全部搜索 | 4.116 | 2.579 | 1.60× | 6.111 | 3.268 |
| 无匹配搜索 | 1.758 | 0.404 | 4.36× | 2.555 | 0.510 |
| 文档大纲（对照） | 2.811 | 2.751 | 1.02× | 3.350 | 3.847 |

small／medium 的精确搜索 p50 分别加速 1.57×／2.12×，无匹配搜索加速 1.74×／2.81×。缓存主要减少重复符号分析，未改善其他语言功能。large 的连续 8 次编辑到补全为 36.534 → 36.553 ms，标签补全为 22.547 → 23.570 ms，诊断为 30.859 → 32.491 ms；保留这些波动，不将本轮解读为普遍加速，p95 也受到机器负载影响。

最初让层次化大纲的首次查询也建立缓存，造成约 10% 的大纲耗时增加；最终改为首次直接返回新树，仅复用已经存在的缓存，反转测量顺序后大纲 p50 恢复到基线附近。

完整样本与环境：[本轮基线 JSON](../benchmarks/results/2026-10-26/round3-before.json)、[本轮优化后 JSON](../benchmarks/results/2026-10-26/round3-after.json)。新增指标改变了脚本指纹，请仅在本轮内配对。

```sh
python3 benchmarks/report.py benchmarks/results/2026-10-26/round3-before.json \
  benchmarks/results/2026-10-26/round3-after.json \
  --output target/benchmarks/round3-report.html
```

本轮整仓回归 3344 项通过、490 项外部工具测试默认跳过；最终缓存范围收窄后，26 项导航／文献符号／语言角色回归、严格全目标 Clippy、release 构建和三档真实 stdio benchmark 再次通过。回归覆盖未保存更新、旧快照、UTF-16、容器、300 多项候选的排序与截断、语言切换及缓存超限后的完整结果。

## 2026-10-26 第五轮：包装宏的数据流保护

在 `17c9233` 补齐命令包装参数的 hook 数据保护后，新增 `--hook-wrappers` 负载：每节调用两层包装宏，将实际参数传入 hook。审阅发现，保护只需字面区间和声明位置，却构造了含完整操作数分析的引用上下文；每轮数据流传播还为无关命令做名称与上下文检查。现在复用较小的源码上下文，并先筛选可能的数据读取器名称；静态 `csuse`／`csname`、分析预算和数据边界保留。

连续编辑会话仍按三档、3 个新进程 ×12 个测量周期、预热 3 周期运行。墙钟出现较大波动：首次大型包装项目改名 p50 为 36.30→38.40 ms（0.95×），交换顺序后的独立复测为 37.51→30.37 ms（1.24×）。早期只筛选名称的中间版本也出现多项未改功能同时变慢，因此不将单组会话或名称筛选单独作为稳定收益的依据。全部原始样本保留。

为进一步比较改名计算，新增 `benchmarks/rename_pairs.py`：两版服务器读取同一个固定项目快照，每次依次发送相同的改名请求，逐对交换顺序；3 组新进程，每组预热 5 对、测量 20 对，共 60 对。Linux 下在启动服务器前绑定同一允许的 CPU，并在打开文件后核验全部线程的亲和性，确保 Tokio 工作线程也受约束。该探测不包含编辑和其他功能，不能替代连续会话；它同时记录墙钟原始样本和服务器 `/proc` CPU 增量。CPU 单样本分辨率为 10 ms，表内为 60 个样本的均值，尤其不根据小项目的少量 CPU tick 宣称优化。

以下每行均按“原保护实现→优化后”排列；反向复测的 JSON 交换了两版的 `before`／`after` 参数。

| 固定快照 | 改名 p50 前／后 ms | p50 倍率 | 服务器 CPU 均值前／后 ms |
| --- | ---: | ---: | ---: |
| small 包装 | 2.44／2.00 | 1.22× | 2.33／2.33 |
| medium 包装 | 11.19／8.51 | 1.31× | 11.50／8.50 |
| large 包装 | 33.15／25.95 | 1.28× | 34.17／26.50 |
| large 包装，交换二进制顺序 | 38.19／30.45 | 1.25× | 39.33／30.33 |
| large 常规 | 23.51／22.59 | 1.04× | 24.33／23.50 |

大型包装的两组逐对墙钟倍率中位数约为 1.26×、1.25×，服务器 CPU 均值下降约 22%–23%。结论限于这项数据保护的改名成本；常规负载变化较小，也不声称其他功能整体加速。

原始数据位于 `benchmarks/results/2026-10-26/round5*.json`。`guard` 为最早基线、`filter` 为仅筛选名称的中间版；`before`／`after` 和 `wrappers` 保存最终连续会话，`wrappers-repeat` 保存反向会话，`pairs` 保存正式配对探测。`pairs-*-main-thread` 及 `pairs-main-thread` 是早期只在启动后绑定主线程的探测，未核验 Tokio 工作线程，单独保留但不用于上述同核结论。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round5-guard \
  --after target/benchmarks/latex-lsp-round5-final --hook-wrappers \
  --output target/benchmarks/pairs.json
python3 benchmarks/report.py \
  benchmarks/results/2026-10-26/round5-wrappers-before.json \
  benchmarks/results/2026-10-26/round5-wrappers-after.json \
  --paired benchmarks/results/2026-10-26/round5-pairs.json \
  --output target/benchmarks/round5-wrappers-report.html
```

保护实现的稳定整仓基线为 319 个目标、3359 项通过、0 失败。优化后再次通过 521 项单元及相关回归、8 项 hook 协议、6 项显式原生测试和全目标严格 Clippy；原生数据保护覆盖 21 种直接／包装／命名调用，核验捕获文本、宏执行次数和实际独立标签编辑后的完整 AUX。CI 加入包装会话与配对探测短测，不设机器相关的性能门槛。dev 清理释放 9.1 GiB，测量结束时可用空间约 20 GiB。

## 2026-10-26 第六轮：环境负载与无关查询的 hook 索引

基线为 `0f26cef`，已保护环境包装与额外参数转发中的 hook 数据。新增 `--hook-environments`，与 `--hook-wrappers` 组合测量：large 含 10 个文件、240046 字节、64 个 xparse 环境和 512 次环境调用，结束正文把开始参数传给 hook。独立标签的全部编辑及文件集合仍经过校验。

最初尝试按文档索引数据流分析中的正文所属定义。同核配对改名的 small／medium／large p50 倍率约为 0.99×／0.96×／0.99×，未显示稳定收益，因此撤回；`round6-owner-index-pairs.json` 保留此次中间实验。随后发现普通标签导航也会建立全部源码的 hook 名称索引。现在先解析光标所在文档，若该消费者解释下没有目标 hook 名称，直接结束此项分析；真正的 hook 名称查询继续扫描完整项目，并保留所有共享消费者的一致性检查。没有增加跨请求缓存。

连续会话为每档 3 个独立进程，每进程预热 3 周期、测量 12 周期。大型组合负载的定义 p50 为 79.95→76.46 ms，悬停 51.00→45.36 ms，改名 47.85→48.81 ms；常规负载定义为 26.77→25.89 ms，改名 19.40→20.30 ms。会话仍有明显波动，不将这些变化解释为所有功能的普遍加速。

`rename_pairs.py` 新增 `--query definition`，核对精确声明位置，沿用启动前同核绑定、打开后全部线程亲和性检查和交替请求。每档为 3 组进程，预热 5 对、测量 20 对。下表始终按“原版→优化版”排列，反向复测文件的 `before`／`after` 则交换了二进制。

| 固定快照 | 定义 p50 前／后 ms | p50 倍率 | CPU 均值前／后 ms |
| --- | ---: | ---: | ---: |
| small 组合 | 4.51／4.34 | 1.04× | 4.83／4.67 |
| medium 组合 | 20.03／18.32 | 1.10× | 21.33／18.50 |
| large 组合 | 77.31／69.55 | 1.11× | 77.83／70.83 |
| large 组合，交换顺序 | 77.40／69.21 | 1.12× | 77.00／70.50 |

两组大型探测的 CPU 均值下降约 8%–9%；CPU tick 为 10 ms，不能根据小项目的少量 tick 宣称收益。结论限于避免无关 hook 索引的导航成本；配对探测没有连续编辑，不能替代完整会话。测量期间未并行运行构建、测试或浏览器渲染。

全部原始数据为 `benchmarks/results/2026-10-26/round6*.json`，合计约 290 KiB：`environments-before/after` 和 `regular-before/after` 为连续会话，`definition-pairs` 及其 `reversed` 为正式配对，`owner-index-pairs` 为已撤回实验。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round6-guard \
  --after target/benchmarks/latex-lsp-round6-origin-first \
  --query definition --hook-wrappers --hook-environments \
  --output target/benchmarks/definition-pairs.json
python3 benchmarks/report.py \
  benchmarks/results/2026-10-26/round6-environments-before.json \
  benchmarks/results/2026-10-26/round6-environments-after.json \
  --paired benchmarks/results/2026-10-26/round6-definition-pairs.json \
  --output target/benchmarks/round6-environments-report.html
```

两种配对查询及环境组合会话的 smoke 通过，CI 同步覆盖。44 项 hook 相关测试、8 项显式原生测试、全目标严格 Clippy、文档、格式及差异检查通过。冻结源码后的整仓回归为 320 个目标、3365 项通过、0 失败、494 项默认跳过，其中完整串行协议为 406 项通过、29 项默认跳过。独立 HTML 的配对表及项目／分位数／指标筛选交互、全部原始样本数量与数值检查通过；预览服务仍返回 `Headless Chrome could not render the page: the browser exited unexpectedly.`，未计为视觉验证通过。

## 2026-10-26 第七轮：环境控制接口与无图片项目

基线为 `b41daa5`，已保护环境生成的开始／结束控制序列直接转发 hook 时的数据。新增 `--hook-interfaces`，每节通过环境生成的控制序列直接调用 hook；它可与 `--hook-wrappers --hook-environments` 组合，CI 的会话与两种配对查询同步覆盖。各项测量继续核验独立标签的完整编辑集合及精确声明位置。

先后尝试跳过安全检查未使用的 hook 提示索引，以及在 hook 名称索引前筛选光标所在的候选字段。大型同核配对收益仅约 1%–3%，均已撤回；保留 `round7-without-hint-index-pairs.json` 和 `round7-definition-pairs.json`，不将它们计为最终优化。

临时分阶段诊断显示，大型定义请求中的文档链接分析约花费 21–22 ms，即使项目没有任何图片，也会建立全项目图片签名和操作数分析。正式优化先检查全部有界 LaTeX 来源，包括 recorder-only 文件：若源码均不含 `includegraphics`，且无 `csuse`／`csname`／`endcsname`，则直接保留每个文档的空调用列表。静态别名链和命令复制必须有最终内置名称，命名构造则一律回退完整分析；不能只检查查询文件，也未增加跨请求缓存。含图片字样或命名构造的项目继续走原分析，因而本轮收益限于符合此条件的项目。正式二进制已移除全部诊断计时。

同核交替定义探测每档 3 组进程、预热 5 对、测量 20 对；启动前绑定 CPU，打开后核验全部线程。下表均按“基线→优化后”排列；反向样本文件交换了二进制参数。

| 固定快照 | 定义 p50 前／后 ms | p50 倍率 | CPU 均值前／后 ms |
| --- | ---: | ---: | ---: |
| small 组合 | 4.70／3.78 | 1.24× | 4.67／3.67 |
| medium 组合 | 20.65／15.83 | 1.30× | 20.17／15.67 |
| large 组合 | 78.08／59.41 | 1.31× | 77.67／58.83 |
| large 组合，交换顺序 | 78.02／60.03 | 1.30× | 77.83／59.67 |

大型两组 CPU 均值下降约 23%–24%；单样本 tick 仍为 10 ms，不按小项目的少量 tick 推断收益。测量期间不并行构建、测试或浏览器渲染。

连续会话仍为三档项目、每档 3 个独立进程、预热 3 周期、测量 12 周期。大型组合为 10 个文件、261082 字节：定义 p50 为 80.36→62.34 ms，悬停 48.23→32.66 ms，连续 8 次编辑到补全 86.57→69.40 ms；改名则为 52.50→55.08 ms，hook 参数提示为 22.39→23.21 ms。常规大型项目的定义为 27.64→19.95 ms、悬停 16.94→10.23 ms、改名 19.75→18.86 ms。本轮未改动改名与 hook 参数提示算法，完整会话仍有波动，不能声称所有操作加速。

原始数据为 `benchmarks/results/2026-10-26/round7*.json`：`interfaces-before/after` 与 `regular-before/after` 保存连续会话，`no-images-definition-pairs` 及其 `reversed` 保存正式配对；另外两个配对文件是已撤回尝试；8 份 JSON 合计 317212 字节，10560 个墙钟原始样本的数量、有限值及中位数校验通过。HTML 的配对表、项目／分位数／指标筛选已用 JS VM 校验；预览仍返回 `Headless Chrome could not render the page: the browser exited unexpectedly.`，未计为视觉验证通过。

```sh
PYTHONDONTWRITEBYTECODE=1 python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round7-guard \
  --after target/benchmarks/latex-lsp-round7-no-images \
  --query definition --hook-wrappers --hook-environments --hook-interfaces \
  --output target/benchmarks/definition-pairs.json
python3 benchmarks/report.py \
  benchmarks/results/2026-10-26/round7-interfaces-before.json \
  benchmarks/results/2026-10-26/round7-interfaces-after.json \
  --paired benchmarks/results/2026-10-26/round7-no-images-definition-pairs.json \
  --output target/benchmarks/round7-interfaces-report.html
```

冻结 Rust 后的完整回归为 320 个目标、3370 项通过、0 失败、495 项默认跳过，其中完整串行协议 407 项通过、29 项默认跳过。新增回归验证未保存提供文件从无图片定义到新增别名、删除和恢复时，图片导航与文件改名同步刷新；既有跨文件、命名、recorder-only、共享消费者及不完整来源回归通过。12 项显式原生测试、全目标严格 Clippy、文档、格式和差异检查通过。dev 产物在验证后清理，保留 release 基线和全部原始测量。

## 2026-10-26 第八轮：自定义 hook 名称改名

功能基线为 `78b9374`，支持完整的自定义 hook 名称编辑，独立检查共享消费者、动态调用、来源及读取器身份。新增 `--hook-renames` 和配对 `--query hook-rename`；主文件查询显式声明，逐项比较全部名称编辑的文件、UTF-16 范围和替换文本。组合负载同时启用命令包装、环境包装和环境生成接口，三档分别检查 36／164／580 项名称编辑。

测量发现完整改名检查重复构造了 `named_references::Context`，再次分析操作数、expl3 数据和正文。名称索引已经在相同项目的每个消费者解释中排除操作数；校验仍要求精确匹配完整索引条目。因此校验改用较小的 `LiteralContext`，保留字面区域、动态调用、接口身份、相关修改、来源与冲突检查，复用已有条目的操作数排除结果。没有新增缓存或跨请求状态。

正式配对使用三组新进程，每组预热 5 对、测量 40 对；启动前同核绑定，打开项目后核验所有线程的 CPU 亲和性。两版交替请求，所有编辑均做完整校验。CPU 为每版 120 次请求的均值，单样本计时粒度为 10 ms。

| 组合项目 | hook 改名 p50：原版 → 优化版 | p50 加速 | CPU 均值：原版 → 优化版 |
| --- | ---: | ---: | ---: |
| small | 2.906 → 2.516 ms | 1.15× | 2.750 → 2.417 ms |
| medium | 13.577 → 10.952 ms | 1.24× | 13.000 → 9.917 ms |
| large | 52.221 → 41.304 ms | 1.26× | 49.333 → 38.750 ms |
| large，交换二进制参数后按原版 → 优化版换算 | 51.940 → 41.003 ms | 1.27× | 49.667 → 38.833 ms |

两组大型配对的 CPU 均值下降约 21%–22%。完整编辑会话使用三组新进程，每组预热 3 个周期、测量 20 个周期；hook 名称改名的 small／medium／large p50 分别从 2.953／13.105／55.001 降为 2.552／10.844／43.607 ms。大型普通标签改名为 59.625 → 59.854 ms，hook 参数提示为 23.811 → 25.906 ms；不据此宣称所有功能加速。会话同时核验未保存主文档、中文／emoji 增量编辑、诊断修复及其他功能结果。

两版的 11 组真实 release 编辑审核结果一致：5 组完整编辑逐项等于手工标记的正确编辑，并保留事件、捕获数据与 AUX；6 组动态或不完整场景均拒绝编辑。新增捕获 hook token 的回归：朴素名称替换会改变数据文本，执行次数和 AUX 却保持相同，索引条目排除检查继续拒绝这种编辑。

原始报告共 250657 bytes（约 245 KiB），9276 个墙钟样本的数量、p50、p95、最大值及配对 CPU 均值已复算：

- [组合会话：原版](../benchmarks/results/2026-10-26/round8-hook-renames-before.json)
- [组合会话：优化版](../benchmarks/results/2026-10-26/round8-hook-renames-after.json)
- [同核配对：三档](../benchmarks/results/2026-10-26/round8-hook-rename-pairs.json)
- [同核配对：交换二进制参数](../benchmarks/results/2026-10-26/round8-hook-rename-pairs-reversed.json)

```sh
python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round8-guard \
  --after target/benchmarks/latex-lsp-round8-index-proof \
  --query hook-rename --hook-wrappers --hook-environments --hook-interfaces \
  --iterations 40 --warmup 5 --runs 3 --output target/benchmarks/round8-hook-rename-pairs.json
```

新指标、三种配对查询、报告交互和两版 release 原生审核通过。最终冻结源码的整仓验证为 321 个目标、3377 项通过、0 失败、497 项默认跳过，完整串行协议为 408 项通过、29 项默认跳过。64 项 hook 专项（含原生）、严格全目标 Clippy、文档、格式及差异检查通过。478 份 Rust 源码与冻结时的校验值一致，两版测量二进制的 SHA 与报告一致。HTML 交互脚本检查通过；两次 headless 预览均因浏览器异常退出失败，不计视觉验证通过。

## 2026-10-26 第九轮显式代码标签与字段候选筛选

功能基线为 `513ab8a`，新增以 `(hook, code label)` 为身份的定义、引用、悬停和补全。负载同时启用 `--hook-labels --hook-wrappers --hook-environments --hook-interfaces --hook-renames`，保留两类改名的全部编辑校验。三档有 17／129／513 个代码块定义及 18／133／521 项引用；代码标签响应逐项核验精确范围，补全核验 hook 隔离及整个后缀的替换。

源码审阅发现，hook 名称与代码标签索引会先为光标文档构造完整操作数上下文，再判断光标是否位于相关字段。优化先解析候选字段；光标未命中时提前返回，有候选的文档才构造上下文，并按候选所属调用检查操作数。字面区域、声明位置、共享消费者一致性、来源预算和改名的完整调用保护均保留，没有新增缓存。已有数据实参、未保存提供文件和共享消费者回归用于检查筛选顺序的改变。

正式配对在 CPU 0 启动两个常驻 release 进程，打开后核验所有服务器线程的亲和性；请求顺序及每组进程启动顺序均交替。每档 3 组进程，每组预热 5 对、测量 30 对，再交换 `--before`／`--after` 完整复测。下表统一按功能基线 → 优化版列出，反向报告本身保留实际参数顺序。

| 查询／项目 | 首组 p50 · ms | 交换后 p50 · ms | 两组倍率 |
| --- | ---: | ---: | ---: |
| 普通定义 · small | 4.562 → 3.919 | 4.572 → 3.925 | 1.16×／1.16× |
| 普通定义 · medium | 18.854 → 17.327 | 18.679 → 17.090 | 1.09×／1.09× |
| 普通定义 · large | 69.861 → 66.269 | 70.783 → 67.401 | 1.05×／1.05× |
| 代码标签定义 · small | 2.796 → 2.432 | 2.686 → 2.364 | 1.15×／1.14× |
| 代码标签定义 · medium | 11.294 → 10.609 | 11.508 → 10.570 | 1.06×／1.09× |
| 代码标签定义 · large | 42.971 → 41.159 | 43.257 → 41.398 | 1.04×／1.04× |

large 普通定义的服务器 CPU 均值分别为 70.222 → 66.667、70.444 → 66.889 ms，均下降约 5.1%；代码标签定义为 41.222 → 39.778、42.778 → 39.889 ms，下降约 3.5%／6.8%。CPU 单样本精度为 10 ms，只比较每档 90 个样本的均值，不据单个样本作结论。

完整连续会话为每档 3 个新进程，各预热 3 个周期、测量 20 个周期。large 的结果如下：

| 操作 | 优化前 p50 · ms | 优化后 p50 · ms |
| --- | ---: | ---: |
| 代码标签定义 | 43.941 | 41.660 |
| 代码标签引用 | 45.412 | 42.848 |
| 代码标签悬停 | 44.771 | 39.019 |
| 代码标签补全 | 46.763 | 40.795 |
| 普通定义 | 72.854 | 73.899 |
| 普通改名 | 63.614 | 61.826 |
| hook 名称改名 | 55.230 | 54.016 |
| hook 参数提示 | 25.365 | 25.243 |
| 连续 8 次编辑到补全 | 81.895 | 75.165 |

配对查询确认小幅稳定收益；连续会话仍有波动，普通定义在本次完整回放中略慢，不能据此声称所有功能加速。测试使用合成论文，关闭发行版发现并使用 pull 诊断，不包含 PDF 编译或外部格式化。

六份正式原始报告合计 350040 bytes（约 342 KiB），11916 个墙钟样本的统计量、CPU 均值、fixture、脚本与二进制 SHA 均已复算：

- [完整会话优化前](../benchmarks/results/2026-10-26/round9-code-labels-before.json)、[优化后](../benchmarks/results/2026-10-26/round9-code-labels-after.json)
- [普通定义配对](../benchmarks/results/2026-10-26/round9-field-candidates-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round9-field-candidates-definition-pairs-reversed.json)
- [代码标签定义配对](../benchmarks/results/2026-10-26/round9-field-candidates-code-label-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round9-field-candidates-code-label-pairs-reversed.json)

```sh
python3 benchmarks/editing.py --server target/benchmarks/latex-lsp-round9-labels \
  --hook-labels --hook-wrappers --hook-environments --hook-interfaces --hook-renames \
  --runs 3 --iterations 20 --warmup 3 --output target/benchmarks/round9-before.json
python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round9-labels \
  --after target/benchmarks/latex-lsp-round9-field-candidates \
  --query hook-definition --hook-labels --hook-wrappers --hook-environments --hook-interfaces \
  --runs 3 --iterations 30 --warmup 5 --output target/benchmarks/round9-pairs.json
```

功能版与优化版均通过实际 release 的四种导航及原生编辑审核：直接、静态命名、纯转发和边界注释的精确定义／引用／悬停保持一致；独立普通标签的实际编辑应用后保持代码事件顺序与完整 AUX。严格全目标 Clippy 已通过，优化版最终冻结全量回归进行中。

交互报告的 JS VM 校验通过，包含三档切换、p50／p95、全部指标及配对表。`html_preview` 仍返回 `Headless Chrome could not render the page: the browser exited unexpectedly.`；未将该预览记为视觉验证通过。

优化版最终冻结验证通过：322 个目标、3385 项通过、0 失败、499 项默认跳过，其中完整串行 stdio 为 409 项通过、29 项默认跳过。73 项含原生 hook 专项、严格全目标 Clippy、文档、格式及差异检查通过；481 份 Rust 源码集合和校验值均一致。普通／hook 会话 smoke 及四种配对查询模式通过；六份正式 JSON 与两版 release SHA 再次校验一致。

## 2026-10-26 第十轮：生命周期标签与撤回的命令预筛选

本轮增加 `--lifecycle-labels`，将逐节注册改成 `AtBeginDocument`，混合直接 `AddToHook{begindocument}`、结束阶段和其他 hook 的干扰。每次查询均核验完整定义／引用与精确补全范围。三档负载分别为 3／6／10 文件、16400／80871／290107 bytes；代码标签定义为 17／129／513 处，引用为 18／133／521 处。

功能基线来自 `b244dcd`，保存为 `target/benchmarks/latex-lsp-round10-lifecycle`，SHA-256 为 `a4daa8704e0585d1cdc543b5c7df1afb274788d02d4b6cb5a43408704114bf0a`。完整会话每档启动 3 个进程，各预热 3 个周期、测量 20 个周期：

| 操作 p50 · ms | small | medium | large |
| --- | ---: | ---: | ---: |
| 普通定义 | 3.943 | 17.414 | 68.709 |
| 生命周期代码标签定义 | 2.429 | 10.562 | 41.865 |
| 生命周期代码标签引用 | 2.516 | 10.771 | 41.736 |
| 生命周期代码标签悬停 | 2.346 | 9.791 | 37.820 |
| 生命周期代码标签补全 | 2.694 | 10.794 | 40.196 |
| 自定义 hook 名称改名 | 2.766 | 11.860 | 46.330 |
| 连续 8 次编辑到补全 | 9.618 | 27.542 | 72.698 |

待优化点是 hook 字段读取器反复检查不相关命令的字面区域和转发身份。测试了三种候选：从当前签名预收集已确认的接口别名；在此基础上先排除不可能的内置名称；以及去掉别名集合，仅把已有的转发判断前移。全部保留后续命名调用、当前内核叶接口、数据角色和共享消费者检查。首版通过 80 项含原生 hook 专项和严格全目标 Clippy，但性能探测未显示收益。

大型项目的探测均在同核的两个常驻进程中交替请求，2 轮、每轮预热 4 次、测量 15 次。每版每项查询有 30 个样本；以下是探测结果，不是完整双顺序性能认证：

| 候选 | 查询 | 基线 p50 → 候选 p50 · ms | 配对比值中位数 |
| --- | --- | ---: | ---: |
| 预收集别名集合 | 普通定义 | 65.797 → 67.136 | 0.98× |
| 别名集合加名称预筛选 | 普通定义 | 66.934 → 68.001 | 0.98× |
| 别名集合加名称预筛选 | 代码标签定义 | 40.201 → 40.639 | 0.99× |
| 直接前移转发判断 | 普通定义 | 66.673 → 67.782 | 0.99× |
| 直接前移转发判断 | 代码标签定义 | 40.681 → 42.288 | 0.97× |

这些小规模探测不足以证明普遍退化，但没有达到保留优化的标准；三种候选均已撤回，Rust 源码与已完成整仓验证的功能提交完全一致。没有新增缓存，也不宣称本轮查询加速。下一轮继续按实际功能负载测量，优先关注重复签名构造，而非增加未证实有效的筛选层。CPU 时钟精度为 10 ms，不据单个样本判断微小变化。

六份原始报告合计 141993 bytes，5178 个墙钟样本的统计量、fixture、CPU 均值及脚本／二进制 SHA 已复算：

- [完整生命周期会话](../benchmarks/results/2026-10-26/round10-lifecycle-before.json)
- [别名集合：普通定义](../benchmarks/results/2026-10-26/round10-filter-definition-rejected.json)
- [名称预筛选：普通定义](../benchmarks/results/2026-10-26/round10-uppercase-definition-rejected.json)、[代码标签定义](../benchmarks/results/2026-10-26/round10-uppercase-label-rejected.json)
- [前移判断：普通定义](../benchmarks/results/2026-10-26/round10-early-meaning-definition-rejected.json)、[代码标签定义](../benchmarks/results/2026-10-26/round10-early-meaning-label-rejected.json)

```sh
python3 benchmarks/editing.py --server target/benchmarks/latex-lsp-round10-lifecycle \
  --lifecycle-labels --hook-wrappers --hook-environments --hook-interfaces --hook-renames \
  --runs 3 --iterations 20 --warmup 3 --output target/benchmarks/round10-lifecycle.json
```

功能版冻结回归为 322 个目标、3391 项通过、0 失败、501 项默认跳过，其中完整串行 stdio 为 410 项通过、29 项默认跳过。80 项含原生 hook 专项、九组实际 release 原生／精确导航审核、严格全目标 Clippy、文档、格式及差异检查通过。测量结束后已恢复功能版 release；不保留失败候选的源码。

## 2026-10-26 第十一轮：全局排序规则与请求内签名复用

功能基线为 `723f611`。新增 `--global-hook-rules`，可与 `--lifecycle-labels` 组合，覆盖局部与全局规则的定义、引用、悬停和补全。三档局部定义为 17／129／513 处，引用为 19／137／529 处；全局定义为 33／257／1025 处，引用为 35／265／1041 处。独立计算全部位置，校验全局补全包含 `contrast` 和 `contribution`，局部定义及补全仅包含本 hook。普通与 hook 名称改名仍检查完整编辑。

源码审阅发现，同一请求中的 hook 名称索引和代码标签索引重复构造项目命令签名。优化在已有的请求级缓存中保存 owned 签名，以 workspace 身份及精确源码作用域区分；每类最多四项，与已有分析共享 8 MiB 计量预算，超限仅重新计算、不继续保留。缓存随请求结束销毁，不跨未保存编辑复用。定义、悬停和补全也进入已有的符号查询缓存，因此同时复用了已有 named／label 分析；本轮结果不能全部归因于签名复用。字面数据、接口含义及所有共享消费者检查保持。

正式配对使用同核两个常驻 release 进程，核验全部线程的亲和性，交替请求顺序；每档三组进程，各预热五对、测量三十对，再交换进程启动顺序完整复测。表内统一按功能基线 → 优化版展示，单位 ms：

| 查询／项目 | 首组 p50 | 交换后 p50 |
| --- | ---: | ---: |
| 普通定义 · small | 4.166 → 4.045 | 4.026 → 3.869 |
| 普通定义 · medium | 18.388 → 18.156 | 18.244 → 18.068 |
| 普通定义 · large | 72.781 → 72.305 | 73.570 → 72.865 |
| 局部标签定义 · small | 2.488 → 2.372 | 2.510 → 2.362 |
| 局部标签定义 · medium | 11.588 → 11.571 | 11.302 → 11.136 |
| 局部标签定义 · large | 45.007 → 43.635 | 45.609 → 44.099 |
| 全局标签定义 · small | 3.017 → 2.859 | 2.645 → 2.479 |
| 全局标签定义 · medium | 12.438 → 12.054 | 12.192 → 12.083 |
| 全局标签定义 · large | 50.043 → 50.096 | 50.703 → 49.496 |

大型局部定义两组 p50 均改善约 3%；普通定义大型约 1%。大型全局定义一组持平／略慢，另一组改善约 2.4%，不能认定稳定改善 5%。其服务器 CPU 均值分别为 47.667 → 46.889、47.222 → 45.889 ms，约下降 1.6%／2.8%；CPU 单样本精度为 10 ms，只比较整档均值。大型控制组的普通改名为 64.291 → 64.344、63.961 → 64.664 ms，没有稳定收益；hook 名称定义为 39.252 → 38.574、39.395 → 38.857 ms，没有发现明显新增开销。

完整连续会话每档三个新进程，各预热三个周期、测量二十个周期。large p50 如下；会话仍有时序波动，不以此替代配对结论，也不宣称全面加速：

| 操作 | 优化前 p50 · ms | 优化后 p50 · ms |
| --- | ---: | ---: |
| 普通定义 | 77.861 | 75.167 |
| 局部标签定义 | 46.114 | 45.197 |
| 全局标签定义 | 55.005 | 49.577 |
| 全局标签引用 | 54.619 | 51.330 |
| 全局标签悬停 | 43.381 | 42.294 |
| 全局标签补全 | 48.821 | 43.494 |
| 普通改名 | 67.656 | 66.727 |
| hook 名称改名 | 54.797 | 51.845 |
| hook 参数提示 | 27.634 | 27.325 |
| 连续八次编辑到补全 | 84.101 | 82.091 |

十二份正式报告合计 487300 bytes，另保留十份初步探测；全部二十二份为 562096 bytes、16236 个墙钟样本，统计量、CPU 均值、fixture、脚本与两版 release SHA 均已复算。测试使用合成论文、pull 诊断，关闭发行版发现，不包含 PDF 编译或外部格式化。

- [完整会话优化前](../benchmarks/results/2026-10-26/round11-global-rules-before.json)、[优化后](../benchmarks/results/2026-10-26/round11-global-rules-after.json)
- [普通定义配对](../benchmarks/results/2026-10-26/round11-query-signatures-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round11-query-signatures-definition-pairs-reversed.json)
- [局部标签定义配对](../benchmarks/results/2026-10-26/round11-query-signatures-hook-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round11-query-signatures-hook-definition-pairs-reversed.json)
- [全局标签定义配对](../benchmarks/results/2026-10-26/round11-query-signatures-global-hook-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round11-query-signatures-global-hook-definition-pairs-reversed.json)
- [普通改名控制](../benchmarks/results/2026-10-26/round11-query-signatures-rename-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round11-query-signatures-rename-pairs-reversed.json)
- [hook 名称定义控制](../benchmarks/results/2026-10-26/round11-query-signatures-hook-name-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round11-query-signatures-hook-name-definition-pairs-reversed.json)

```sh
python3 benchmarks/editing.py --server target/benchmarks/latex-lsp-round11-global-rules \
  --global-hook-rules --lifecycle-labels --hook-wrappers --hook-environments \
  --hook-interfaces --hook-renames --runs 3 --iterations 20 --warmup 3 \
  --output target/benchmarks/round11-before.json
python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round11-global-rules \
  --after target/benchmarks/latex-lsp-round11-query-signatures \
  --query global-hook-definition --lifecycle-labels --hook-wrappers \
  --hook-environments --hook-interfaces --runs 3 --iterations 30 --warmup 5 \
  --output target/benchmarks/round11-pairs.json
```

实际优化版 release 的八组全局规则及九组生命周期审核通过，包含精确导航、补全、普通引用实际编辑后的原生事件／完整 AUX 和保留名称拒绝。交互报告已发布，JS VM 校验覆盖三档、p50／p95、全部指标及全局配对标签；`html_preview` 返回 `Headless Chrome could not render the page: the browser exited unexpectedly.`，未计视觉验证通过。旧生命周期报告也可继续生成。

优化版最终冻结回归通过：322 个目标、3401 项通过、0 失败、504 项默认跳过，其中完整串行 stdio 为 411 项通过、29 项默认跳过。90 项含原生 hook 专项、严格全目标 Clippy、文档、格式及差异检查通过；481 份 Rust 源码集合和 hash 与冻结清单一致，两版 release SHA 与原始报告一致。新增所有权、作用域／workspace 隔离、请求后刷新、嵌套 panic 恢复和缓存预算回退回归通过。测量后清理本包 dev 产物，保留 release 基线、依赖和原始样本。

## 2026-10-26 第十二轮：默认标签设置与跨分析签名复用

功能基线为 `ee618a4`。新负载在每节执行 Push／Set／省略标签注册／Pop，并查询设置字段的定义、引用、悬停和完整后缀补全；与全局规则、生命周期、命令／环境包装及环境生成接口组合。三档分别为 3／6／10 文件、20208／110247／405435 bytes。局部定义／引用为 17／36、129／269、513／1049，全局和默认设置为 33／52、257／397、1025／1561。所有位置独立计算；只计当前已认证的显式注册，不把省略注册计作已支持的定义。普通引用及 hook 名称的完整编辑继续逐项检查。

源码审阅确认，hook 索引已共享命令签名，但同一请求的命名引用分析与标签声明数据证明仍为相同作用域重复构造签名。优化统一按精确作用域和 URI 次序选取 LaTeX 源码，三种分析共用已有的请求内签名缓存。无查询缓存时保留借用键，避免强制复制；查询内保留的结果继续拥有全部键。各自的源码／命令预算、数据角色与共享消费者检查保持，缓存仍每类最多四项、共享 8 MiB 计量预算，不跨请求复用。

正式配对将两个常驻 release 进程绑定同一 CPU，并核验全部线程；交替请求，每档三组进程、各预热五对并测量三十对，然后交换启动顺序完整复测。普通定义／改名均在同一组合项目内查询普通引用键，默认设置和 hook 名称定义作为控制。下表统一按功能基线 → 优化版列出，单位 ms；反向 JSON 保留其实际参数顺序。

| 查询／项目 | 首组 p50 | 交换后 p50 |
| --- | ---: | ---: |
| 普通定义 · small | 4.453 → 4.016 | 4.379 → 4.044 |
| 普通定义 · medium | 22.040 → 21.413 | 21.764 → 21.183 |
| 普通定义 · large | 88.565 → 83.916 | 87.936 → 83.216 |
| 默认标签设置定义 · small | 2.832 → 2.817 | 2.724 → 2.708 |
| 默认标签设置定义 · medium | 14.695 → 14.557 | 14.325 → 13.929 |
| 默认标签设置定义 · large | 59.237 → 58.318 | 57.713 → 57.061 |
| 普通改名 · large | 76.919 → 74.244 | 76.998 → 74.586 |
| hook 名称定义 · large | 47.737 → 46.987 | 47.037 → 46.685 |

大型普通定义两组 p50 分别改善约 5.2%／5.4%，服务器 CPU 均值为 87.778 → 83.111、87.889 → 83.000 ms，约下降 5.3%／5.6%。大型普通改名 p50 改善约 3.5%／3.1%，CPU 均值为 75.333 → 71.889、75.000 → 72.667 ms。默认字段和 hook 名称定义的变化较小，不把普通定义的收益推广到所有 hook 查询。CPU 单样本精度为 10 ms，只比较整档均值。

完整连续会话每档启动三个新进程，各预热三个周期、测量二十个周期，large p50 如下。全局／默认定义、默认引用、hook 名称改名及输入突发仍有波动，不能据配对普通定义的收益宣称全面加速：

| 操作 | 优化前 p50 · ms | 优化后 p50 · ms |
| --- | ---: | ---: |
| 普通定义 | 91.309 | 87.321 |
| 普通引用 | 94.126 | 92.173 |
| 普通标签补全 | 69.195 | 67.228 |
| 局部 hook 标签定义 | 53.510 | 53.446 |
| 全局 hook 标签定义 | 60.883 | 62.348 |
| 默认标签设置定义 | 61.784 | 62.238 |
| 默认标签设置引用 | 67.547 | 67.914 |
| 默认标签设置悬停 | 51.748 | 50.662 |
| 默认标签设置补全 | 55.508 | 53.724 |
| 普通改名 | 78.838 | 75.812 |
| hook 名称改名 | 61.988 | 62.763 |
| hook 参数提示 | 32.391 | 31.923 |
| 连续八次编辑到补全 | 97.531 | 97.869 |

十份正式报告为 465282 bytes、15516 个墙钟样本；另保留两份初探，全部十二份为 475482 bytes、15636 个样本。统计量、CPU 均值、fixture、脚本与两版 release SHA 均已复算。完整普通／组合 smoke 和七种配对查询模式通过。测试关闭发行版发现、采用 pull 诊断，不包含 PDF 编译或外部格式化；结果依赖合成负载与机器。

- [完整会话优化前](../benchmarks/results/2026-10-26/round12-default-settings-before.json)、[优化后](../benchmarks/results/2026-10-26/round12-default-settings-after.json)
- [普通定义配对](../benchmarks/results/2026-10-26/round12-project-signatures-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round12-project-signatures-definition-pairs-reversed.json)
- [默认设置定义配对](../benchmarks/results/2026-10-26/round12-project-signatures-default-hook-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round12-project-signatures-default-hook-definition-pairs-reversed.json)
- [普通改名控制](../benchmarks/results/2026-10-26/round12-project-signatures-rename-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round12-project-signatures-rename-pairs-reversed.json)
- [hook 名称定义控制](../benchmarks/results/2026-10-26/round12-project-signatures-hook-name-definition-pairs.json)、[交换顺序](../benchmarks/results/2026-10-26/round12-project-signatures-hook-name-definition-pairs-reversed.json)
- [普通定义初探](../benchmarks/results/2026-10-26/round12-project-signatures-definition-probe.json)、[默认设置初探](../benchmarks/results/2026-10-26/round12-project-signatures-default-hook-definition-probe.json)

```sh
python3 benchmarks/editing.py --server target/benchmarks/latex-lsp-round12-default-settings \
  --default-hook-labels --global-hook-rules --lifecycle-labels --hook-wrappers \
  --hook-environments --hook-interfaces --hook-renames --runs 3 --iterations 20 \
  --warmup 3 --output target/benchmarks/round12-before.json
python3 benchmarks/rename_pairs.py \
  --before target/benchmarks/latex-lsp-round12-default-settings \
  --after target/benchmarks/latex-lsp-round12-project-signatures \
  --query definition --default-hook-labels --global-hook-rules --lifecycle-labels \
  --hook-wrappers --hook-environments --hook-interfaces --runs 3 --iterations 30 \
  --warmup 5 --output target/benchmarks/round12-pairs.json
```

优化版六项请求缓存测试、严格全目标 Clippy 及七组实际 release／原生审核通过；后者检查精确字段、补全、代码块移除及普通引用实际编辑后的事件和完整 AUX。最终冻结整仓回归为 322 个目标、3408 项通过、0 失败、505 项默认跳过，其中完整串行 stdio 为 412 项通过、29 项默认跳过。96 项含原生 hook 专项、文档、格式及差异检查通过；481 份 Rust 源码集合与 hash 匹配冻结清单，两版 release SHA 与全部原始报告一致。测量后清理本包 dev 产物，保留依赖、release 基线及原始样本。

交互报告已发布；普通定义、默认字段配对及上一轮全局报告各通过十八种档位／统计量／指标组合的 JS VM 校验。`html_preview` 仍返回 `Headless Chrome could not render the page: the browser exited unexpectedly.`，未计视觉验证通过。
