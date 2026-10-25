# TeX 发行版与宏包

服务器启动后在后台调用 `kpsewhich --expand-path=$TEXMFDBS`，读取返回目录的 `ls-R` 文件名数据库；发行版发现和文件监听注册不会阻塞文档打开、编辑或正向搜索。数据库就绪前，相关补全和文件跳转先使用项目内文件及内置元数据。`usepackage`、`RequirePackage`、文档类和 `bibliographystyle` 的补全与文件跳转会查找发行版；同名项目文件优先。删除或移动后已不存在的文件不会作为结果返回。发行版源文件不会自动加入项目符号、重命名或诊断范围。

```json
{
  "latex": {
    "distribution": {
      "enabled": true,
      "executable": "kpsewhich",
      "roots": []
    }
  }
}
```

`roots` 是额外的 TeX 树根目录，每个目录需包含 `ls-R`，优先于自动发现目录。没有 `kpsewhich` 或数据库时仍可使用项目内功能。发现进程最多运行两秒；最多读取 64 个目录、每个数据库 32 MiB、共 100000 个文件名。修改配置或重启服务器可刷新数据库。

随服务器发布的 `assets/packages.json` 提供可扩展的宏包描述和命令参数元数据。当前涵盖 amsmath、mathtools、amsfonts、amssymb、booktabs、geometry、graphicx 的常用命令，以及 hyperref、fontspec、tikz、pgfplots、biblatex 的宏包说明。项目源码中的静态宏包加载及元数据声明的依赖关系决定命令补全、参数提示和悬停文档；用户声明优先。悬停提供 texdoc.org 文档链接。

这是一份精选元数据，不会执行宏包或自动分析发行版全部宏定义。动态加载、条件加载和文档类内部的隐式宏包依赖可能需要显式加载或补充元数据。`\input` 发行版内部文件的任意搜索路径暂不由此索引提供。
