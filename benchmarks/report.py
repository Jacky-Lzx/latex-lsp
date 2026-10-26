#!/usr/bin/env python3
"""Build a standalone, interactive HTML comparison from two editing benchmark reports."""

import argparse
import json
from pathlib import Path


HTML = r"""<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>TeXquill continuous editing performance</title>
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
<h2>Continuous editing: feature latency comparison</h2>
<p id="method"></p>
<div id="paired"></div>
<nav><label>Project <select id="scale"></select></label>
<label>Metric <select id="stat"><option value="p50_ms">p50</option><option value="p95_ms">p95</option><option value="max_ms">Maximum</option></select></label>
<label><input type="checkbox" id="all"> Show all metrics</label></nav>
<p id="fixture"></p>
<table><thead><tr><th>Feature · ms</th><th><span class="key"></span>Before</th><th><span class="key new"></span>After</th><th>Speedup</th></tr></thead><tbody id="rows"></tbody></table>
<p>Real release processes, LSP over stdio, and UTF-16 incremental edits, including Chinese input, multiline paste, unsaved root-document changes, and reference fixes. Feature results are checked after every edit. Distribution discovery is disabled and diagnostics use pull requests. PDF builds and external formatting are excluded. Bars share one millisecond scale; shorter is faster. Speedup = before / after; values below 1 indicate a slowdown. This is a synthetic paper workload; results depend on the machine and workload.</p>
<script>
const data=__DATA__;
const names={"initialize":"Initialization","open":"Open buffers","edit.prose":"Prose edit","edit.preamble":"Root-document edit","edit.reference":"Reference edit",
"completion.labels":"Label completion","completion.citations":"Citation completion","completion.commands":"Command completion","hover":"Hover","definition":"Definition","references":"References",
"rename":"Cross-file rename","signature":"Signature help","symbols":"Document outline","folding":"Folding","semantic.full":"Full semantic tokens","semantic.delta.cached":"Cached semantic delta",
"diagnostic":"Recomputed diagnostics","diagnostic.cached":"Cached diagnostics","burst8.to_completion":"8-edit burst → completion",
"workspace.symbols.exact":"Workspace symbols · exact","workspace.symbols.all":"Workspace symbols · all","workspace.symbols.missing":"Workspace symbols · no match","signature.hook":"Hook signature help","rename.hook":"Custom hook name rename",
"definition.hook_label":"Local code labels · Definition","references.hook_label":"Local code labels · References","hover.hook_label":"Local code labels · Hover","completion.hook_label":"Local code labels · Completion",
"definition.relative_hook_label":"Relative code labels · Definition","references.relative_hook_label":"Relative code labels · References","hover.relative_hook_label":"Relative code labels · Hover","completion.relative_hook_label":"Relative code labels · Completion",
"definition.document_hook_slot":"Document-level slots · Definition","references.document_hook_slot":"Document-level slots · References","hover.document_hook_slot":"Document-level slots · Hover","completion.document_hook_slot":"Document-level slots · Completion",
"definition.default_hook_label":"Default label settings · Definition","references.default_hook_label":"Default label settings · References","hover.default_hook_label":"Default label settings · Hover","completion.default_hook_label":"Default label settings · Completion",
"definition.global_hook_label":"Global code labels · Definition","references.global_hook_label":"Global code labels · References","hover.global_hook_label":"Global code labels · Hover","completion.global_hook_label":"Global code labels · Completion"};
const selected=["completion.labels","completion.citations","hover","definition","references","definition.hook_label","references.hook_label","hover.hook_label","completion.hook_label","definition.relative_hook_label","references.relative_hook_label","hover.relative_hook_label","completion.relative_hook_label","definition.document_hook_slot","references.document_hook_slot","hover.document_hook_slot","completion.document_hook_slot","definition.default_hook_label","references.default_hook_label","hover.default_hook_label","completion.default_hook_label","definition.global_hook_label","references.global_hook_label","hover.global_hook_label","completion.global_hook_label","rename","rename.hook","signature","signature.hook","workspace.symbols.exact","workspace.symbols.all","workspace.symbols.missing","diagnostic","burst8.to_completion"];
const scale=document.getElementById('scale'), stat=document.getElementById('stat'), all=document.getElementById('all');
for(const key of Object.keys(data.after.scenarios)){const option=document.createElement('option');option.value=key;option.textContent=key;scale.append(option)}
scale.value=Object.hasOwn(data.after.scenarios,'large')?'large':scale.options[0].value;
const p=data.after.parameters;
document.getElementById('method').textContent=`${p.runs} independent processes × ${p.iterations} measured cycles; each process first runs ${p.warmup} warmup cycles. Each cycle performs 11 edits and queries features sequentially. Initialization and opening are measured separately.${p.hooks?' Includes kernel hooks and signature-help checks.':''}${p.hook_wrappers?' Each section includes nested wrappers to measure argument-data forwarding guards.':''}${p.hook_environments?' Each section includes environment wrappers whose end bodies forward begin arguments to hooks.':''}`;
if(p.hook_interfaces)document.getElementById('method').textContent+=' Each section also directly calls environment-generated hook-forwarding control sequences.';
if(p.hook_renames)document.getElementById('method').textContent+=' Complete cross-file edits for custom hook names are checked individually.';
if(p.hook_labels)document.getElementById('method').textContent+=' All code-label definitions, references, hover results, and full-suffix completions are checked individually.';
if(p.lifecycle_labels)document.getElementById('method').textContent+=' Begin and end phases remain independent.';
if(p.relative_hook_labels)document.getElementById('method').textContent+=' Relative labels resolve only against an immediately adjacent literal Push; ranges and completions preserve relative spelling.';
if(p.document_hook_slots)document.getElementById('method').textContent+=' Document-level slots link only explicit registrations and removals for their own hook, excluding ordering rules and defaults.';
if(p.default_hook_labels)document.getElementById('method').textContent+=' Default label settings link explicit registrations across hooks; registrations with omitted labels are not counted as certified definitions.';
if(p.global_hook_rules)document.getElementById('method').textContent+=' Global rules span multiple hooks; local definitions retain their own scope.';
if(data.paired){
 const report=data.paired,host=document.getElementById('paired'),note=document.createElement('p');
 const query=({paired_definition:'definitions',paired_hook_rename:'hook name rename',paired_hook_definition:'local code-label definitions',paired_relative_hook_definition:'relative code-label definitions',paired_document_hook_definition:'document-level slot definitions',paired_default_hook_definition:'default label definitions',paired_global_hook_definition:'global code-label definitions',paired_hook_name_definition:'hook name definitions',paired_rename:'rename'})[report.kind];
 const binding=report.environment.cpu===null?'no CPU affinity':`bound to CPU ${report.environment.cpu} before launch, with every thread checked after opening`;
 const precision=report.environment.cpu_tick_ms===null?'CPU not recorded':`CPU values are means across all samples, with ${report.environment.cpu_tick_ms} ms per-sample precision`;
 note.textContent=`Paired ${query} probes on fixed snapshots: ${report.parameters.runs} fresh process pairs, ${report.parameters.warmup} warmup pairs and ${report.parameters.iterations} measured pairs per run. Requests alternate between versions; ${binding}. Wall-clock values below are p50; ${precision}. This probe excludes continuous edits; the full session below retains runtime variability.`;
 host.append(note);const table=document.createElement('table');table.className='pairs';
 const heading=document.createElement('tr');for(const value of ['Project',`${query} p50 · ms`,'Speedup','Mean CPU · ms']){const cell=document.createElement('th');cell.textContent=value;heading.append(cell)}table.append(heading);
 for(const [scale,result] of Object.entries(report.scenarios)){
  const a=result.wall_ms.before.p50_ms,b=result.wall_ms.after.p50_ms,cpu=result.cpu_mean_ms,row=document.createElement('tr');
  for(const value of [scale,`${a.toFixed(2)} → ${b.toFixed(2)}`,`${(a/b).toFixed(2)}×`,cpu.before===null?'not recorded':`${cpu.before.toFixed(2)} → ${cpu.after.toFixed(2)}`]){const cell=document.createElement('td');cell.textContent=value;row.append(cell)}table.append(row);
 }host.append(table);
}
function render(){
 const before=data.before.scenarios[scale.value],after=data.after.scenarios[scale.value],f=after.fixture;
 document.getElementById('fixture').textContent=`${f.files} files · ${(f.bytes/1024).toFixed(1)} KiB · ${data.after.environment.platform}`;
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
