# Task8实际演示工程生成入口

物品可视布局的完整升级入口：

```text
python -X utf8 -B tools/v2-demo/geometry.py <全新bundle目录> --evidence <本批私有证据目录>
```

从原保存r2真实导入，生成新模块精确版本、源/汇六槽及设备命名锚点槽，同步四条PortRef、路由与实例版本；显式采用简化LOD依赖，避免“源包有模块、冻结工程未采用”。随后实际Host创建新预测Run、保存、另起新Host重开，保留原Run/冻结字节，调用真实External Engine/ExecutionProjector并解析实际Host读取的素材。只使用`C:/Users/fujunye/AppData/Local/Temp/sf-v2-demo-geometry-20261027/target`独立缓存，无Root target/Native/真实云。

当前正式保存示例为`resources/v2-demo/geometry-v4/project/`r4（三个Run）；本轮完整重现从原r2直接生成新r3，RunId由实际新进程生成，不要求与已保存示例相同。实际四步退出码均0，见`.work/v2-implementation-20261026/demo-geometry/reproduction-evidence/generation-commands.json`。旧生成命令及下文r2说明继续有效。

从软件工程根运行。输出必须是新bundle父目录中的新project目录；已有工程、映射及旧Run拒绝覆盖。输入只依赖随包resources/v2-demo/source，无需旧.work源工程，不重建或改变模型。

```powershell
python -X utf8 -B tools/v2-demo/generate.py <新的bundle父目录>/project --evidence <本批私有证据目录>
```

入口用新C:专用target编译sf-commands/sf-simulation普通库，实际执行：原Host公开import_package(open_project)→adopt_imported_draft→set_declared_parameter→run_scenario/Task/get_run→save_project；随后另起新Host进程重开，读取原Run/冻结输入/模块与read_media_asset素材，并调用真实Engine::new_external。最后复用生产presentation的GLB/场景解码器和已安装Pillow像素适配器，无Native/端口/云/GPU。所有步骤exit0才签收。

输出project/是实际ProjectArchive；父目录另含candidate-bindings.json与profile-bindings.json，均为非秘密映射。源模块/GLB由writer原样保留，10/15秒仅在实例与真实parameter_sources上由Host修改；预测仍按v1固定A运行。

已有包可只检查：

```powershell
python -X utf8 -B tools/v2-demo/audit-package.py resources/v2-demo <本批私有审计回执.json>
```

本批已实际重跑到.work/v2-implementation-20261026/demo-package/reproduction/project，四步命令均exit0；未覆盖随包示例。新输出中的RunId由新Host实际生成，不承诺与已保存示例RunId相同。
