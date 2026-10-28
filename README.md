# 超级工厂——开源工业生产设计、仿真与云端执行平台

简称：超级工厂。

**介绍文档和视频创作：** [2026-10-28 完整材料入口](docs/creative-materials-20261028/README.md)，包含介绍册、可编辑源、截图、任务8说明、脚本与实录。


面向工业方案建模、彩排和云端数据驱动生产的软件在环演示。作者：傅钧烨。
发布候选：`SF-v2.0.0-20261027-R1`。原创代码采用MIT；第三方及素材许可保持原样。

## 下载与查看

- [Windows运行包](https://github.com/JunyeFu/superfactory-v2/releases/download/v2.0.0/SuperFactory-v2.0.0-windows.zip)
- [188秒实际演示](https://github.com/JunyeFu/superfactory-v2/releases/download/v2.0.0/superfactory-v2.0.0-demo.mp4)
- [作品介绍PDF](https://github.com/JunyeFu/superfactory-v2/releases/download/v2.0.0/superfactory-v2.0.0-introduction.pdf)
- [作品介绍可编辑源](https://github.com/JunyeFu/superfactory-v2/releases/download/v2.0.0/superfactory-v2.0.0-introduction-source.zip)
- [固定版本与发布说明](https://github.com/JunyeFu/superfactory-v2/releases/tag/v2.0.0)

运行包包含可信CPython 3.14.9设备运行时。解压完整目录，双击
`start_superfactory.cmd`，打开 `demo/project-original-r4`；录制后的
`demo/project-final-recorded`保留5份执行和3个原Run。需要Windows11 x64及WebView2。
云端连接由部署者提供私有配置；配置、凭据和设备journal不随仓库或运行包分发。

## 核心流程

保留中央三维工作台、左右停靠区和共用时钟的时间轴。设备、物品、资源、工艺、
物流、专业计算、模块创作及四格式报告继续消费同一业务内核。

任务8新增真实HTTPS/PostgreSQL、两个独立软件设备、设备观测与DaaS、确定性派工
及执行回执。只有Native Host/Engine提交业务效果；准备冻结输入后须显式启动。
停止A新接单后，原任务继续完成，尚未开始的工作由合法设备承接。
云端生产在一个面板提供设备、数据和执行三个页签。

实际普通2.0.0运行包结果：

|场景|固定修订及Host时点|结果|
|---|---|---|
|正常生产|R349 / 51.747秒|6件，A3/B3|
|A原任务开始后停止新接单|R535 / 90.515秒|6件，A1/B5，无新增A尝试|
|新OS进程重开|新Host、原r4|5份执行快照及3个原Run保留，历史控制不恢复|
|完整筛选导出|固定F与G000选择|UI/MCP各10000行、50页，与原PG独立预期一致|

托管百万行/60次完整查询的p95为原始63ms、聚合1641ms；十设备各1Hz持续1800秒，
18000条的采样、首次请求、ACK、PG及分钟聚合已核对。原100份请求至正确物理画面
同钟样本的p95保守上界1402.7524ms。生成负载与实际产量分开。
原0.1.0验证候选与最终普通包各保留自己的身份和结果；软件在环时间不代表真机效率。

## 从源码运行

本仓库根目录就是原项目的“软件工程”目录。工具链：Node24.15.0/npm12.0.1、
Rust1.97.0、MSVC Windows构建工具及WebView2。依赖按锁文件恢复。

```powershell
npm ci --ignore-scripts
npm run build:web
cargo build -p sf-desktop-host -p sf-mcp-bridge --release --features sf-desktop-host/custom-protocol --locked
```

发行时复制Tauri声明的全部资源及可信设备运行时；可使用
`tools/release-preparation/package_v2.py`，不能只取EXE。
源码带有原模块、场景、必要输入及建模素材，体积较大；不附npm/Cargo编译缓存。

## 云端部署与配置

见 [Sealos部署说明](tools/cloud/sealos/README.md) 和
[运行与演示说明](docs/05_开源与参赛/release-preparation/v2/运行与演示说明.md)。
Native私有连接文件为 `%APPDATA%/org.superfactory.planner/v2-cloud/connection.json`，
其读取、网关及设备身份由实际部署者配置。Renderer不持有云密钥。
当前桌面组件2.0.0与已验云组件0.1.0分别固定，云镜像摘要记录于
`docs/release/2.0.0/verification.json`；演示云服务可由部署者保留或停止。

## 范围及历史资料

本版覆盖任务8的单工序、两设备、六件软件在环场景。真实硬件、通用接入向导、
全局在线优化、500设备全部质量通过不属于本轮完成声明。
历史设计、原验证矩阵和旧候选材料保留原身份；不把未执行的全产品正式矩阵写为通过。

MIT及第三方/素材许可见 [LICENSE](LICENSE)、[NOTICE](NOTICE) 和各原声明。
研究下载原文、私有连接、设备journal、数据库连接串和编译缓存不在发布范围。
