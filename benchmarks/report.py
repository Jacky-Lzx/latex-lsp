#!/usr/bin/env python3
"""Build a standalone, interactive HTML comparison from two editing benchmark reports."""

import argparse
import json
from pathlib import Path


HTML = r"""<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>LaTeX LSP 连续编辑性能</title>
<style>
:root{font-family:system-ui,sans-serif;color:var(--foreground,#18212c);--old:#8796aa;--new:#17a589}
body{margin:0}h2{font-size:20px;margin:0 0 8px}p{color:var(--muted-foreground,#596576);font-size:13px;line-height:1.6}
nav{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:20px 0}
select{font:inherit;padding:6px;border:1px solid var(--border,#d2d8df);border-radius:5px;background:var(--background,white);color:inherit}
label{font-size:13px}table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums}
th,td{padding:10px 4px;text-align:right;border-bottom:1px solid var(--border,#d2d8df);font-size:13px}
th:first-child,td:first-child{text-align:left;width:48%}th{font-weight:500;color:var(--muted-foreground,#596576)}
.bars{display:grid;gap:3px;margin-top:6px}.bar{height:4px;border-radius:2px;min-width:1px;background:var(--old)}
.bar.new{background:var(--new)}.gain{font-weight:600;color:var(--success,#13856e)}.loss{color:var(--destructive,#be4955)}
.key{display:inline-block;width:12px;height:4px;background:var(--old);vertical-align:middle;margin-right:4px}.key.new{background:var(--new)}
.pairs th:first-child,.pairs td:first-child{width:auto}
@media(max-width:420px){th,td{font-size:12px;padding:9px 2px}th:first-child,td:first-child{width:42%}}
</style>
<h2>连续编辑：功能延迟对比</h2>
<p id="method"></p>
<div id="paired"></div>
<nav><label>项目 <select id="scale"></select></label>
<label>指标 <select id="stat"><option value="p50_ms">p50</option><option value="p95_ms">p95</option><option value="max_ms">最大值</option></select></label>
<label><input type="checkbox" id="all"> 显示全部指标</label></nav>
<p id="fixture"></p>
<table><thead><tr><th>功能 · 毫秒</th><th><span class="key"></span>优化前</th><th><span class="key new"></span>优化后</th><th>速度倍率</th></tr></thead><tbody id="rows"></tbody></table>
<p>真实 release 进程、LSP stdio、UTF-16 增量编辑；包含中文输入、多行粘贴、未保存的主文档变更和引用错误修复。每次编辑后检查功能结果。发行版发现关闭，使用 pull 诊断；不包含 PDF 编译和外部格式化。条长采用同一毫秒比例尺，越短越快。速度倍率＝优化前／优化后，小于 1 表示变慢。合成论文负载，结果依赖机器与负载。</p>
<script>
const data=__DATA__;
const names={"initialize":"初始化","open":"打开缓冲区","edit.prose":"正文编辑处理","edit.preamble":"主文档编辑处理","edit.reference":"引用编辑处理",
"completion.labels":"标签补全","completion.citations":"文献补全","completion.commands":"命令补全","hover":"悬停","definition":"定义跳转","references":"查找引用",
"rename":"跨文件重命名","signature":"参数提示","symbols":"文档大纲","folding":"折叠","semantic.full":"完整语义令牌","semantic.delta.cached":"缓存语义 delta",
"diagnostic":"重新计算诊断","diagnostic.cached":"缓存诊断","burst8.to_completion":"连续 8 次编辑 → 补全",
"workspace.symbols.exact":"工作区符号 · 精确搜索","workspace.symbols.all":"工作区符号 · 全部搜索","workspace.symbols.missing":"工作区符号 · 无匹配","signature.hook":"钩子参数提示","rename.hook":"自定义 hook 名称改名",
"definition.hook_label":"局部代码标签 · 定义","references.hook_label":"局部代码标签 · 引用","hover.hook_label":"局部代码标签 · 悬停","completion.hook_label":"局部代码标签 · 补全",
"definition.relative_hook_label":"相对代码标签 · 定义","references.relative_hook_label":"相对代码标签 · 引用","hover.relative_hook_label":"相对代码标签 · 悬停","completion.relative_hook_label":"相对代码标签 · 补全",
"definition.document_hook_slot":"文档级槽 · 定义","references.document_hook_slot":"文档级槽 · 引用","hover.document_hook_slot":"文档级槽 · 悬停","completion.document_hook_slot":"文档级槽 · 补全",
"definition.default_hook_label":"默认标签设置 · 定义","references.default_hook_label":"默认标签设置 · 引用","hover.default_hook_label":"默认标签设置 · 悬停","completion.default_hook_label":"默认标签设置 · 补全",
"definition.global_hook_label":"全局代码标签 · 定义","references.global_hook_label":"全局代码标签 · 引用","hover.global_hook_label":"全局代码标签 · 悬停","completion.global_hook_label":"全局代码标签 · 补全"};
const selected=["completion.labels","completion.citations","hover","definition","references","definition.hook_label","references.hook_label","hover.hook_label","completion.hook_label","definition.relative_hook_label","references.relative_hook_label","hover.relative_hook_label","completion.relative_hook_label","definition.document_hook_slot","references.document_hook_slot","hover.document_hook_slot","completion.document_hook_slot","definition.default_hook_label","references.default_hook_label","hover.default_hook_label","completion.default_hook_label","definition.global_hook_label","references.global_hook_label","hover.global_hook_label","completion.global_hook_label","rename","rename.hook","signature","signature.hook","workspace.symbols.exact","workspace.symbols.all","workspace.symbols.missing","diagnostic","burst8.to_completion"];
const scale=document.getElementById('scale'), stat=document.getElementById('stat'), all=document.getElementById('all');
for(const key of Object.keys(data.after.scenarios)){const option=document.createElement('option');option.value=key;option.textContent=key;scale.append(option)}
scale.value=Object.hasOwn(data.after.scenarios,'large')?'large':scale.options[0].value;
const p=data.after.parameters;
document.getElementById('method').textContent=`${p.runs} 个独立进程 × ${p.iterations} 个测量周期；每个进程先预热 ${p.warmup} 个周期。每周期 11 次编辑，顺序查询各项功能。初始化、打开单独统计。${p.hooks?' 包含内核钩子及其参数提示校验。':''}${p.hook_wrappers?' 每节含嵌套包装调用，测量参数数据传递保护。':''}${p.hook_environments?' 每节含环境包装，由结束正文把开始参数传入 hook。':''}`;
if(p.hook_interfaces)document.getElementById('method').textContent+=' 每节还直接调用环境生成的 hook 转发控制序列。';
if(p.hook_renames)document.getElementById('method').textContent+=' 逐项核验自定义 hook 的完整跨文件名称编辑。';
if(p.hook_labels)document.getElementById('method').textContent+=' 代码标签逐项核验全部定义、引用、悬停和完整后缀补全。';
if(p.lifecycle_labels)document.getElementById('method').textContent+=' 开始与结束阶段保持独立。';
if(p.relative_hook_labels)document.getElementById('method').textContent+=' 相对标签只按紧邻字面 Push 解析，范围与补全保留相对拼写。';
if(p.document_hook_slots)document.getElementById('method').textContent+=' 文档级代码槽只关联本 hook 的显式注册和移除，不关联排序规则或默认设置。';
if(p.default_hook_labels)document.getElementById('method').textContent+=' 默认标签设置关联各 hook 的显式注册，省略注册不计入已认证定义。';
if(p.global_hook_rules)document.getElementById('method').textContent+=' 全局规则关联多个 hook，局部定义保持自身作用域。';
if(data.paired){
 const report=data.paired,host=document.getElementById('paired'),note=document.createElement('p');
 const query=({paired_definition:'普通定义',paired_hook_rename:'hook 名称改名',paired_hook_definition:'局部代码标签定义',paired_relative_hook_definition:'相对代码标签定义',paired_document_hook_definition:'文档级代码槽定义',paired_default_hook_definition:'默认标签设置定义',paired_global_hook_definition:'全局代码标签定义',paired_hook_name_definition:'hook 名称定义',paired_rename:'普通改名'})[report.kind];
 const binding=report.environment.cpu===null?'未绑定 CPU':`启动前绑定 CPU ${report.environment.cpu}，打开后核验所有线程`;
 const precision=report.environment.cpu_tick_ms===null?'未采集 CPU':`CPU 为所有样本的均值，单样本精度 ${report.environment.cpu_tick_ms} ms`;
 note.textContent=`固定快照的配对${query}探测：${report.parameters.runs} 组新进程，每组预热 ${report.parameters.warmup} 对，测量 ${report.parameters.iterations} 对；两版交替请求，${binding}。下表墙钟为 p50；${precision}。它不包含连续编辑；下方完整会话保留了运行时波动。`;
 host.append(note);const table=document.createElement('table');table.className='pairs';
 const heading=document.createElement('tr');for(const value of ['项目',`${query} p50 · ms`,'速度倍率','CPU 均值 · ms']){const cell=document.createElement('th');cell.textContent=value;heading.append(cell)}table.append(heading);
 for(const [scale,result] of Object.entries(report.scenarios)){
  const a=result.wall_ms.before.p50_ms,b=result.wall_ms.after.p50_ms,cpu=result.cpu_mean_ms,row=document.createElement('tr');
  for(const value of [scale,`${a.toFixed(2)} → ${b.toFixed(2)}`,`${(a/b).toFixed(2)}×`,cpu.before===null?'未采集':`${cpu.before.toFixed(2)} → ${cpu.after.toFixed(2)}`]){const cell=document.createElement('td');cell.textContent=value;row.append(cell)}table.append(row);
 }host.append(table);
}
function render(){
 const before=data.before.scenarios[scale.value],after=data.after.scenarios[scale.value],f=after.fixture;
 document.getElementById('fixture').textContent=`${f.files} 个文件 · ${(f.bytes/1024).toFixed(1)} KiB · ${data.after.environment.platform}`;
 const keys=all.checked?Object.keys(after.metrics):selected.filter(k=>Object.hasOwn(after.metrics,k));
 const max=Math.max(...keys.flatMap(k=>[before.metrics[k][stat.value],after.metrics[k][stat.value]]));
 const rows=document.getElementById('rows');rows.replaceChildren();
 for(const key of keys){
  const a=before.metrics[key][stat.value],b=after.metrics[key][stat.value],row=document.createElement('tr');
  const label=document.createElement('td');label.textContent=names[key]||key;
  const bars=document.createElement('div');bars.className='bars';
  for(const [value,cls] of [[a,'bar'],[b,'bar new']]){const bar=document.createElement('div');bar.className=cls;bar.style.width=`${value/max*100}%`;bars.append(bar)}
  label.append(bars);row.append(label);
  for(const value of [a,b]){const cell=document.createElement('td');cell.textContent=value.toFixed(2);row.append(cell)}
  const ratio=document.createElement('td');ratio.className=a>=b?'gain':'loss';ratio.textContent=`${(a/b).toFixed(2)}×`;row.append(ratio);rows.append(row);
 }
}
scale.onchange=stat.onchange=all.onchange=render;render();
</script></html>"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--paired", type=Path, help="include a matching fixed-snapshot paired query report")
    args = parser.parse_args()
    before, after = (json.loads(path.read_text()) for path in [args.before, args.after])
    for key in ["schema", "workload_sha256", "parameters"]:
        if before[key] != after[key]:
            parser.error(f"reports differ in {key}")
    for scale, scenario in after["scenarios"].items():
        if before["scenarios"][scale]["fixture"] != scenario["fixture"]:
            parser.error(f"{scale}: fixtures differ")
        if before["scenarios"][scale]["metrics"].keys() != scenario["metrics"].keys():
            parser.error(f"{scale}: metrics differ")
    # Samples remain in the JSON; the page only needs the summary statistics.
    for report in [before, after]:
        for scenario in report["scenarios"].values():
            scenario.pop("samples_ms", None)
    paired = json.loads(args.paired.read_text()) if args.paired else None
    if paired:
        if paired.get("kind") not in {"paired_rename", "paired_definition", "paired_hook_rename", "paired_hook_definition", "paired_default_hook_definition", "paired_document_hook_definition", "paired_relative_hook_definition", "paired_global_hook_definition", "paired_hook_name_definition"}:
            parser.error("paired input must be a supported paired query report")
        for label, report in [("before", before), ("after", after)]:
            if paired["servers"][label]["sha256"] != report["environment"]["server_sha256"]:
                parser.error(f"paired {label} binary differs")
        for scale, scenario in paired["scenarios"].items():
            if after["scenarios"][scale]["fixture"] != scenario["fixture"]:
                parser.error(f"paired {scale} fixture differs")
            scenario.pop("wall_samples_ms", None)
            scenario.pop("cpu_samples_ms", None)
    data = json.dumps({"before": before, "after": after, "paired": paired}, ensure_ascii=False).replace("<", r"\u003c")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(HTML.replace("__DATA__", data))


if __name__ == "__main__":
    main()
