from pathlib import Path
from xml.etree.ElementTree import Element, SubElement, ElementTree
import subprocess

ROOT = Path(__file__).resolve().parent
CLI = Path(r'D:\MathModelingTools\software\drawio-31.3.2\draw.io.exe')
INK, TEAL, SOFT, LINE, GRAY = '#172E3A','#137C89','#E9F2F4','#D7E2E7','#607580'

def build(name, nodes, edges, width=1000, height=230):
    file = Element('mxfile', host='app.diagrams.net')
    page = SubElement(file, 'diagram', name=name, id=name)
    model=SubElement(page,'mxGraphModel', dx=str(width),dy=str(height),grid='0',page='0',pageWidth=str(width),pageHeight=str(height))
    root=SubElement(model,'root'); SubElement(root,'mxCell',id='0');SubElement(root,'mxCell',id='1',parent='0')
    for ident,text,x,y,w,h,kind in nodes:
        fill,stroke,color = (TEAL,TEAL,'#FFFFFF') if kind=='primary' else (SOFT,LINE,INK)
        style=f'rounded=0;whiteSpace=wrap;html=0;fontFamily=Microsoft YaHei;fontSize=21;fontColor={color};fillColor={fill};strokeColor={stroke};spacing=14;align=center;verticalAlign=middle;'
        if kind=='label': style=f'text;whiteSpace=wrap;html=0;fontFamily=Microsoft YaHei;fontSize=18;fontColor={GRAY};strokeColor=none;fillColor=none;align=center;'
        cell=SubElement(root,'mxCell',id=ident,value=text,style=style,vertex='1',parent='1')
        SubElement(cell,'mxGeometry',x=str(x),y=str(y),width=str(w),height=str(h),attrib={'as':'geometry'})
    for i,(src,dst,label) in enumerate(edges):
        cell=SubElement(root,'mxCell',id=f'e{i}',value=label,style=f'edgeStyle=orthogonalEdgeStyle;rounded=0;html=0;endArrow=block;strokeWidth=2;strokeColor={TEAL};fontFamily=Microsoft YaHei;fontSize=17;fontColor={GRAY};labelBackgroundColor=#FFFFFF;',edge='1',parent='1',source=src,target=dst)
        SubElement(cell,'mxGeometry',relative='1',attrib={'as':'geometry'})
    path=ROOT/'diagrams'/f'{name}.drawio'
    ElementTree(file).write(path,encoding='utf-8',xml_declaration=True)
    result=subprocess.run([str(CLI),'-x','-f','png','-e','-s','2','-b','12','-o',str(path.with_suffix('.drawio.png')),str(path)],capture_output=True,text=True,encoding='utf-8',errors='replace',creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode or not path.with_suffix('.drawio.png').exists(): raise RuntimeError(result.stdout+result.stderr)
    print(name, 'exported',flush=True)

build('01_产业课题接续',[
('a','广东省科技计划\n重点领域研发计划',10,25,230,90,'primary'),
('b','母项目\n工业软件集成与工业互联网',285,25,280,90,'normal'),
('c','研究任务 8\n设备数据 · 应用模型 · 调度',610,25,355,90,'normal'),
('d','超级工厂：任务延伸与平台组合',285,165,680,70,'primary'),
('l','母项目编号\n2019B010120001',10,155,230,75,'label')], [('a','b',''),('b','c',''),('c','d','')],height=250)

build('02_完整工作链',[(f'phase{i}',t,10+i*202,20,180,100,'primary' if i==0 else 'normal') for i,t in enumerate(['目标与条件\n订单 · 场地\n班次','建立工程\n设备 · 工艺\n物流','运行观察\n物品 · 资源\n等待','修改比较\n同订单\n同观察窗口','保存交付\n报告\n可编辑工程'])],[(f'phase{i}',f'phase{i+1}','') for i in range(4)],height=140)

build('03_两画布与显示层次',[
('a','流水线画布',10,10,190,60,'primary'),('b','详细\n设备与物品状态',250,10,220,100,'normal'),
('c','简化\n设备轮廓与连接',510,10,220,100,'normal'),('d','总览\n产线与区域聚合',770,10,220,100,'normal'),
('e','多厂房画布',10,165,190,60,'primary'),('f','基地 A\n部件供给',250,150,220,100,'normal'),('g','基地 B\n总装',510,150,220,100,'normal'),('h','基地 C\n包装交付',770,150,220,100,'normal'),
('l','菜单切换',10,85,190,60,'label')],[('b','c',''),('c','d',''),('f','g',''),('g','h','')],height=265)

build('04_专业成果接入',[
('a','结构与尺寸\n受支持纸型 / 参数',10,20,205,105,'normal'),
('b','受限计算\n成型 · 开合\n直线取物',265,20,225,105,'primary'),
('c','专业成果\n结果状态\n制造层输出',540,20,215,105,'normal'),
('d','用途与版本认可',540,185,215,80,'normal'),
('e','包装工位绑定与消费',10,185,480,80,'primary'),
('l','结果可为：通过、不满足、未判定',10,125,480,45,'label')],[('a','b',''),('b','c',''),('c','d',''),('d','e','')],width=800,height=280)

build('05_Agent工作方式',[
('a','用户目标与场地\n补充必要条件',10,15,210,100,'normal'),('b','查询模板与九类库\n选用适用模块',265,15,225,100,'normal'),
('c','调用平台真实操作\n创建可编辑草案',535,15,225,100,'primary'),('d','运行与读取结果\n解释等待与限制',805,15,225,100,'normal'),
('e','人工接管：修改同一工程 → 再运行 → 导出报告',265,180,765,75,'primary'),
('l','工作方式示意\n不是客户端运行截图',10,175,210,85,'label')],[('a','b',''),('b','c',''),('c','d',''),('d','e','')],width=1050,height=270)

build('06_软件在环链路',[
('a','本地 Host / Engine\n维护生产事实',15,20,265,115,'primary'),('b','云端数据服务\n保存、转发观测与命令',365,20,300,115,'normal'),('c','独立软件设备\n执行命令并回报',750,20,265,115,'normal'),
('d','云端回执\n开始 / 完成 / 异常',365,205,300,90,'normal'),('e','Host接纳回执\n画布 · 时间轴 · 报告',15,205,265,90,'normal'),('f','软件设备回报\n对应工作身份',750,205,265,90,'normal')],[('a','b',''),('b','c',''),('c','f',''),('f','d',''),('d','e','')],width=1040,height=320)

build('07_贡献与接手',[
('a','选择一项能力\n设备 · 工艺\n模板 · 规则',10,15,245,95,'normal'),('b','准备可用模块\n定义 · 单位 · 许可 · 示例',310,15,300,95,'primary'),('c','提出与审阅变更\nIssue / PR / 使用检查',665,15,300,95,'normal'),
('d','随版本交付 → 用户放入工程 → 运行并反馈',310,175,655,75,'primary'),('l','社区参与方式\n新项目按入口与样例推进',10,165,245,90,'label')],[('a','b',''),('b','c',''),('c','d','')],height=270)
