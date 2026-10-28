# 超级工厂——开源工业生产设计、仿真与云端执行平台

简称：**超级工厂**。整理日期：2026-10-28。

本目录供介绍文档、产品介绍册和演示视频创作使用，集中提供可编辑文字、真实截图、机制图、视频脚本与实际录制素材。当前公开运行版本仍见 [v2.0.0 发布页](https://github.com/JunyeFu/superfactory-v2/releases/tag/v2.0.0)。本次上传更新创作材料。

## 从哪里开始

1. 阅读[作品介绍工作稿](introduction/作品介绍_工作稿.md)，确定产品定位、用户任务和完整工作链。项目全称以上方为准，页面及旁白可使用简称。
2. 查看[18页介绍册审阅 PDF](introduction/完整介绍册_v01/超级工厂_专业产品介绍册_20261028_审阅版.pdf)及[编辑和生成说明](introduction/完整介绍册_v01/README.md)。正文、排版 Python、七张 draw.io 图、截图、逐页预览都在同目录；既有 PDF 和图像保留原版本标题，下一次排版时统一采用新全称。
3. 阅读[任务8技术答复](task8/任务8答复正文.md)及[PDF](task8/超级工厂_任务8实现说明与技术答复_20261028.pdf)，理解设备上云、DaaS、建模决策和回执驱动生产的联系。
4. 视频制作从[完整演示视频脚本](video/design/演示视频脚本.md)开始，结合[完整制作与叙事设计](video/design/完整制作与叙事设计.md)及该目录中的 JSON、运镜和制作适配文件。目标为6分钟、15段，以四设备双工序订单的维护与接续生产为主线；它是制作设计，最终片尚待补录与制作。
5. 素材使用前对照[事实与缺项](事实与缺项.md)、[首批实录说明](video/design/首批实录与录制状态.md)及[补录与已观察到的问题](video/design/补充实录与当前阻塞.md)。

## 介绍文档素材

- [制作与表达规范](introduction/制作与表达规范.md)、[外观设计规范](introduction/外观设计规范.md)、[编制方案](introduction/编制方案.md)。
- [产业背景与任务8关联说明](introduction/产业背景与任务8关联说明.md)。其中引用的本地原文位置用于追溯；用户提供的完整母项目截图及第三方参考作品未随此次公开上传。可公开引用入口见说明正文。
- [真实软件截图](introduction/完整介绍册_v01/assets/)、[可编辑机制图](introduction/完整介绍册_v01/diagrams/)、[逐页预览](introduction/完整介绍册_v01/preview/)。
- 介绍册内两设备六件的既有云实例、视频设计内四设备双工序案例以及本地纸包装实录分别按原版本引用。

## 可以直接用于剪辑的本地实录

|文件|内容|
|---|---|
|[01-template-and-production.mp4](video/footage/01-template-and-production.mp4)|模板建厂与运行|
|[02-factory-and-timeline.mp4](video/footage/02-factory-and-timeline.mp4)|全景和时间轴|
|[03-device-production.mp4](video/footage/03-device-production.mp4)|设备连续加工回放|
|[04-save-project.mp4](video/footage/04-save-project.mp4)|保存工程|
|[05-new-process-reopen.mp4](video/footage/05-new-process-reopen.mp4)|新进程重开|
|[05b-restored-run.mp4](video/footage/05b-restored-run.mp4)|恢复运行结果|
|[06-professional-calculation.mp4](video/footage/06-professional-calculation.mp4)|受限专业计算|

上述素材来自稳定程序、模板1.6.0。原批次记录为8类订单、100件接受、162道制造作业及119次运输；92台为场景设备数。具体适用范围见实录说明。

另附[109秒粗剪预览](video/roughcut/superfactory-native-demo-roughcut-01.mp4)、[SRT字幕](video/roughcut/captions.srt)和[ASS字幕](video/roughcut/captions.ass)，用于快速了解已有画面。云端旧版实录可从[已发布188秒演示](https://github.com/JunyeFu/superfactory-v2/releases/download/v2.0.0/superfactory-v2.0.0-demo.mp4)下载；四设备维护恢复片段的最终同候选镜头仍按脚本待补项制作。

## 编辑约定

材料保留原来源、实例与版本，图像和旧 PDF 未重新制作。设计文档中的 F:/、D:/ 等路径是原工作环境位置；本目录已经携带的剪辑素材以本页相对链接为准。生成介绍册需要安装其 README 所列 Python 库、字体与 draw.io，并将脚本里的工具路径改成本机路径。

原创文字、代码和机制图遵循仓库许可证；第三方、素材及引用按原 LICENSE、NOTICE 和各自来源说明处理。
