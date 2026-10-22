# 长度诊断与修复

[返回文档索引](README.md) · [寄存器与长度](registers.md)

etoolbox 报告 `Length '\measuer' undefined` 时，服务器会核对完整的 `deflength` 调用和日志源码回显，精确标出长度控制序列，并提供拼写修复。例如：

```tex
\newlength{\measure}
\deflength{\measuer}{2pt+3pt}
```

选择 ``Replace with length `\measure` `` 后，第二行变为 `\deflength{\measure}{2pt+3pt}`。支持裸控制序列、花括号包裹的控制序列、跨行表达式，以及同一行的多次调用；修复保留表达式和赋值前缀。

## 候选与边界

拼写建议来自标准长度及当前项目中静态的 `newlength`、`newskip`、`newdimen`、`skipdef`、`dimendef` 声明，也识别字面 `csname` 声明。普通宏、计数/盒子/muskip 寄存器、重复声明和被其他类型覆盖的名称不作为长度候选；其他论文的声明不参与修复。

宏展开、动态或不完整操作数、命令被重定义、日志回显无法唯一确认时保留整行错误。精确诊断记录错误发生时的宏来源，提供修复前重新核对源码、候选类型及参数边界；支持版本化编辑和旧客户端，过期修复会被拒绝。

## 协议与验证

精确诊断附带 `undefined-length` 代码及名称数据，例如 `{"length":"measuer"}`。通用来源校验见[编译诊断](diagnostics.md#分析范围与来源确认)，默认、协议及真实三引擎回归命令见[测试说明](testing.md#未定义长度诊断)。
