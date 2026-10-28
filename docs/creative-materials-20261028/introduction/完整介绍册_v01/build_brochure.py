"""A4 landscape reading brochure. Run with the document Python environment.
All text remains vector text. Screenshot crops are PDF clipping operations;
original screenshots and native draw.io sources are retained beside this file.
"""
from pathlib import Path
from xml.sax.saxutils import escape
import json, re, math
from reportlab.pdfgen import canvas
from reportlab.lib.pagesizes import A4, landscape
from reportlab.lib.colors import HexColor, white
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import Paragraph, Table, TableStyle
from reportlab.lib.styles import ParagraphStyle
from PIL import Image

ROOT=Path(__file__).resolve().parent
OUT=ROOT/'超级工厂_专业产品介绍册_20261028_审阅版.pdf'
W,H=landscape(A4); M=40; CW=W-2*M
INK='#172E3A'; TEAL='#137C89'; PAPER='#F5F8FA'; MUTED='#607580'; LINE='#D7E2E7'; SOFT='#E9F2F4'; AMBER='#AF701D'
pdfmetrics.registerFont(TTFont('YaHei',r'C:\Windows\Fonts\msyh.ttc',subfontIndex=0))
pdfmetrics.registerFont(TTFont('YaHeiB',r'C:\Windows\Fonts\msyhbd.ttc',subfontIndex=0))
pdfmetrics.registerFontFamily('YaHei',normal='YaHei',bold='YaHeiB',italic='YaHei',boldItalic='YaHeiB')
C=canvas.Canvas(str(OUT),pagesize=(W,H),pageCompression=1)
C.setTitle('超级工厂｜可编辑的工业模拟与生产评估平台')
C.setAuthor('超级工厂项目组'); C.setSubject('专业产品介绍册 · 阅读审阅版 · 2026-10-28')
PAGES=[]; BOXES=[]; ASSETS=[]

def md(s):
    # Preserve public links, use a compact source label for local references.
    s=escape(s).replace('&lt;br/&gt;','<br/>')
    s=re.sub(r'\[([^\]]+)\]\(([^)]+)\)',lambda m: '<link href="'+m[2].replace('&amp;','&amp;')+'" color="'+TEAL+'">'+m[1]+'</link>' if m[2].startswith('http') else m[1],s)
    return re.sub(r'\*\*(.+?)\*\*',r'<b>\1</b>',s)

def para(s,x,y,w,size=11.1,leading=None,color=INK,bold=False,maxh=None,align=0):
    style=ParagraphStyle('p',fontName='YaHeiB' if bold else 'YaHei',fontSize=size,leading=leading or size*1.53,textColor=HexColor(color),wordWrap='CJK',spaceAfter=0,alignment=align)
    p=Paragraph(md(s),style); _,h=p.wrap(w,H)
    if maxh is not None and h>maxh+.01: raise ValueError(f'Page {len(PAGES)} text exceeds box {h:.1f}>{maxh}: {s[:45]}')
    if y-h<42: raise ValueError(f'Page {len(PAGES)} text below body: {s[:45]} -> {y-h:.1f}')
    p.drawOn(C,x,y-h)
    BOXES.append({'page':len(PAGES),'text':re.sub('<[^>]*>','',s)[:50],'x':round(x,1),'top':round(H-y,1),'width':round(w,1),'height':round(h,1),'bottom':round(H-y+h,1)})
    return y-h

def text(s,x,y,size=10,color=INK,bold=False):
    C.setFillColor(HexColor(color)); C.setFont('YaHeiB' if bold else 'YaHei',size);C.drawString(x,y,s)

def rect(x,y,w,h,fill=PAPER,stroke=None):
    C.setFillColor(HexColor(fill));C.setStrokeColor(HexColor(stroke or fill));C.setLineWidth(.7);C.rect(x,y,w,h,fill=1,stroke=int(stroke is not None))

def line(x1,y1,x2,y2,color=LINE,width=.8):
    C.setStrokeColor(HexColor(color));C.setLineWidth(width);C.line(x1,y1,x2,y2)

def begin(title,category='产品与工作方式',sub=None):
    n=len(PAGES)+1;PAGES.append({'page':n,'title':title,'category':category})
    rect(0,0,W,H,'#FFFFFF')
    rect(M,H-34,16,3,TEAL);text('超级工厂  /  '+category,M+26,H-35,9,MUTED)
    text('产品介绍册',W-93,H-35,9,MUTED)
    C.bookmarkPage(f'p{n}');C.addOutlineEntry(f'{n:02d}  {title}',f'p{n}',0,False)
    if n>1:
        para(title,M,H-58,CW,25,31,bold=True,maxh=64)
        if sub: para(sub,M,H-103,CW,10.5,15.2,MUTED,maxh=32)

def end(source=''):
    if source: para(source,M,65,CW,8,11.2,MUTED,maxh=22.5)
    line(M,35,W-M,35)
    text('SUPER FACTORY  /  阅读审阅版 · 2026.10.28',M,21,8,MUTED)
    C.setFont('YaHei',8);C.setFillColor(HexColor(MUTED));C.drawRightString(W-M,21,f'{len(PAGES):02d} / 18')
    C.showPage()

def pic(filename,x,y,w,h,crop=None,category='软件实拍',border=True):
    path=ROOT/filename;im=Image.open(path);iw,ih=im.size
    if crop is None: crop=(0,0,iw,ih)
    l,t,r,b=crop; scale=min(w/(r-l),h/(b-t)); dw=(r-l)*scale;dh=(b-t)*scale
    xx=x+(w-dw)/2; yy=y+(h-dh)/2
    C.saveState();p=C.beginPath();p.rect(xx,yy,dw,dh);C.clipPath(p,stroke=0,fill=0)
    C.drawImage(str(path),xx-l*scale,yy-(ih-b)*scale,width=iw*scale,height=ih*scale,mask='auto');C.restoreState()
    if border:
        C.setStrokeColor(HexColor(LINE));C.setLineWidth(.7);C.rect(xx,yy,dw,dh,fill=0,stroke=1)
    ASSETS.append({'page':len(PAGES),'file':filename,'crop':list(crop),'kind':category})
    return (xx,yy,dw,dh)

def diagram(name,x,y,w,h): return pic('diagrams/'+name+'.drawio.png',x,y,w,h,category='编辑示意图；非软件运行截图',border=False)

def note(s,x,y,w):
    rect(x,y-43,w,43,SOFT);para(s,x+12,y-10,w-24,10.3,15,TEAL,maxh=31)

def height(s,w,size=11.1):
    p=Paragraph(md(s),ParagraphStyle('m',fontName='YaHei',fontSize=size,leading=size*1.53,wordWrap='CJK'))
    return p.wrap(w,H)[1]+9

def columns(paragraphs,y,bottom=85,size=11.1):
    paragraphs=[p for p in paragraphs if p.strip()]; w=(CW-28)/2
    heights=[height(p,w,size) for p in paragraphs]
    split=min(range(1,len(paragraphs)),key=lambda k:abs(sum(heights[:k])-sum(heights[k:]))) if len(paragraphs)>1 else 1
    for col,group in enumerate([paragraphs[:split],paragraphs[split:]]):
        yy=y
        for p in group:
            yy=para(p,M+col*(w+28),yy,w,size,maxh=yy-bottom)-9

def table(headers,rows,widths,x,y,font=10.4):
    def cell(t,bold=False): return Paragraph(md(t),ParagraphStyle('t',fontName='YaHeiB' if bold else 'YaHei',fontSize=font,leading=font*1.45,textColor=HexColor('#FFFFFF' if bold else INK),wordWrap='CJK'))
    data=[[cell(t,True) for t in headers]]+[[cell(str(t)) for t in row] for row in rows]
    tb=Table(data,colWidths=widths,hAlign='LEFT')
    tb.setStyle(TableStyle([('BACKGROUND',(0,0),(-1,0),HexColor(TEAL)),('ROWBACKGROUNDS',(0,1),(-1,-1),[HexColor(PAPER),white]),('VALIGN',(0,0),(-1,-1),'TOP'),('LEFTPADDING',(0,0),(-1,-1),10),('RIGHTPADDING',(0,0),(-1,-1),10),('TOPPADDING',(0,0),(-1,-1),7),('BOTTOMPADDING',(0,0),(-1,-1),7),('LINEBELOW',(0,0),(-1,0),.6,HexColor(TEAL)),('LINEBELOW',(0,1),(-1,-1),.35,HexColor(LINE))]))
    _,hh=tb.wrapOn(C,sum(widths),H)
    if y-hh<80:raise ValueError(f'table overflow page{len(PAGES)}')
    tb.drawOn(C,x,y-hh);return y-hh

raw=(ROOT/'正文.md').read_text(encoding='utf-8-sig')
sections={}
for match in re.finditer(r'### (\d+) (.*?)\n(.*?)(?=\n### |\n## 编制方法|\Z)',raw,re.S):
    parts=[p.strip() for p in match[3].strip().split('\n\n')]
    sections[int(match[1])]={'title':match[2],'body':parts}

def body(i):
    parts=sections[i]['body'].copy()
    if i==1: parts=[p.replace('[背景关系与原文依据](产业背景与任务8关联说明.md)','') for p in parts]
    if i==10: parts=[p.replace('完整技术过程见[任务8实现说明](对外答复/超级工厂_任务8实现说明与技术答复_20261028.pdf)。','对应技术资料列于第18页来源[8]。') for p in parts]
    return parts

# 01 — cover: the machine shown is an actual native GLB view, not an AI image.
begin('先把工厂方案运行一遍，再决定怎样建。','专业产品介绍')
rect(343,80,W-343,H-120,PAPER)
pic('assets/B60-105-zoom-minus960.jpg',357,132,448,369,crop=(620,355,1500,1080),border=False)
text('超级工厂',M,439,37,bold=True)
para('可编辑的工业模拟\n与生产评估平台'.replace('\n','<br/>'),M,405,300,17.7,26,bold=True)
para('先把工厂方案运行一遍，\n再决定怎样建。'.replace('\n','<br/>'),M,329,288,18.8,29,TEAL,bold=True)
para('在桌面画布上组织设备、工艺与物流，沿时间轴理解生产和等待，再交付评估结果与可编辑工程。',M,251,276,11.4,18,maxh=83)
para('实际软件素材 · 原生设备模型静态查看',367,119,427,8.5,12,MUTED)
line(M,148,315,148)
para('产业课题来源\n广东省重点领域研发计划项目研究任务8延伸，结合工厂流水线模拟与评估交付。'.replace('\n','<br/>'),M,135,277,9.8,15,maxh=58)
end('Windows桌面平台  ·  可编辑工程  ·  模块扩展  ·  Agent操作入口')

# 02 — orientation and reading guide.
begin('一份工程，连接搭建、运行、判断与接手','先看全貌','从人工编辑或Agent草案出发，最后仍得到能检查、能继续修改的工程。')
pic('assets/C12-reopened-4000-contact.jpg',M,174,510,279,crop=(0,0,2202,1352))
para('实际桌面工作台：工程对象、设备状态、属性与时间轴处在同一工作区。',M,160,510,9.4,14,MUTED,maxh=29)
x=580
for y,num,title,desc in [(444,'03–05','为什么使用','课题来源、用户问题与完整任务'),(365,'06–12','怎样工作','九类定义、两画布、专业模块与执行'),(286,'13–16','价值怎样成立','结果、贡献、开源及采用条件'),(207,'17–18','怎样接手','交付方式、公开入口及参考资料')]:
    text(num,x,y,12,TEAL,True);para(title,x+64,y+12,157,13.5,19,bold=True);para(desc,x,y-18,220,10.4,15,MUTED,maxh=32)
note('阅读提示：纸箱厂案例用于解释任务；设备历史回看与任务8软件在环是分别标注的实际实例。',M,113,CW)
end('图源：已发布v2.0.0介绍源包，C12保存运行重开后的4.0秒画面。')

# 03 — background.
begin('把工厂方案，变成可以运行和修改的工程','背景与定位','以通用工业平台承载工厂方案，纸包装提供首个深入的专业领域。')
diagram('01_产业课题接续',M,281,CW,163)
para('产业关系示意：用户确认本作品与母项目任务8的延伸关系；公开资料核对母项目与承担单位。',M,272,CW,9,13,MUTED)
columns(body(1),244,bottom=87,size=10.9)
end('来源：[1] 广东省科技厅项目清单；[2] 专项指南；[3] 用户提供的主要研究内容原文。详见第18页。')

# 04 — decision, not market-size rhetoric.
begin('问题是：这份方案能否完成这份订单？','用户与需求','同一台设备放入不同的工艺、物料与资源条件，可能得到不同的交付结果。')
for x,num,title,desc in [(M,'01','空间放得下','设备、产线、物流与厂房边界'),(M+260,'02','过程接得上','前后顺序、齐套、人员与运输'),(M+520,'03','结果说得清','接受产出、完成时刻与等待原因')]:
    rect(x,314,242,127,PAPER);text(num,x+15,410,12,TEAL,True);para(title,x+15,392,212,15,22,bold=True);para(desc,x+15,357,212,10.5,16,MUTED,maxh=34)
columns(body(2),288,bottom=105)
note('采用价值需要沿任务验证：能否帮助用户提出、解释并交接一份可修改方案。',M,113,CW)
end('来源：[4] 西门子南京数控工厂中文白皮书公开简介；其案例效益不归属于本作品。')

# 05 — workflow, five outputs and one transparent calculation.
begin('从场地与产量目标，走到可接手的方案','完整任务','说明案例：给定场地，评估每天交付一万个合格纸箱的方案。')
diagram('02_完整工作链',M,339,CW,102)
note('需求换算示例：8小时 × 3,600秒 ÷ 10,000件 = 2.88秒／合格件。它是需求节奏，不是实测产能。',M,322,CW)
columns(body(3)[1:],260,bottom=86,size=10.8)
end('本页为自行设计的任务说明，非一万件纸箱实际运行记录。场地、班次和目标须由真实用户确认。')

# 06 — nine concepts, exact user vocabulary.
begin('九类定义，组成同一份可运行的工厂模型','平台的组成','模块需要有业务含义、输入输出和实际消费者；外观由美工层表达。')
rows=[['物品','零件、半成品、成品','被加工、装配、检验、包装或跨厂交换'],['固定设备','工位及设备','运行时位置固定；编辑时可重新布置'],['资源','人员、夹具等有限能力','按阶段占用与释放，影响能否开工'],['任务','目标数量及释放条件','引用物品与工艺，形成运行目标'],['工艺','步骤、投入产出与关系','约束工作顺序、齐套及处理条件'],['物流','移动与交接','把物品送至下一工位或厂房'],['厂房','生产单元与输入输出','组织内部布局和跨厂供应'],['计算','专业规则及其结果','将特定用途成果交给业务工位'],['美工','图像、动画、模型','表达业务对象、机构和工作阶段']]
y=table(['概念','定义什么','怎样参与运行'],rows,[92,224,CW-316],M,447,font=10)
paras=[p for p in body(4)[2:] if not p.startswith('|')]
columns(paras,y-18,bottom=85,size=10.4)
end('设备动画呈现生产过程；物品归属、资源占用和产量由业务运行维护。九类分类沿用项目现行主线。')

# 07 — layers are not separate application states.
begin('两张画布，查看同一工程的不同尺度','画布与交互','常用流水线画布；菜单切换多厂房画布。单击进入详情，滚轮只改变显示精度。')
diagram('03_两画布与显示层次',M,252,CW,193)
para('交互结构示意。上排是同一厂房的三档显示；下排是跨厂交换，不表示新增的第三张常态画布。',M,242,CW,9.2,13,MUTED)
columns(body(5),216,bottom=87,size=10.8)
end('连线需表达物品、数量、交付时刻及运输／接收条件；画布切换沿用同一工程与运行状态。')

# 08 — the strongest real visual evidence, two times of the SAME saved Run.
begin('看清“它在等什么”，才知道该改哪里','实际运行回看','同一份已保存运行，两个观察时刻：等待日历条件 → 进入工作阶段。')
w=(CW-18)/2
for x,f,label,summary in [(M,'C12-reopened-1500-wait.jpg','1.5秒｜等待日历条件','进度0%；尚未持有工作资源。'),(M+w+18,'C12-reopened-4000-contact.jpg','4.0秒｜进入执行','进度50%；持有L1-D01-CAP工作资源。')]:
    text(label,x,437,12,TEAL,True)
    pic('assets/'+f,x,241,w,183,crop=(530,475,1775,1010))
    para(summary,x,231,w,9.7,14,MUTED,maxh=29)
paragraphs=body(6)[:3]
columns(paragraphs,201,bottom=85,size=10.6)
end('来源：[8] C12历史保存运行；1.5秒和4.0秒为同一次Run回看，不是方案修改前后的收益对照。')

# 09 — substantive professional capability without implying arbitrary shapes.
begin('专业设计的结果，要能进入包装工位','纸包装专业模块','从受支持的纸结构出发，经用途与版本认可，再接入业务消费。')
diagram('04_专业成果接入',M,248,460,197)
rect(526,257,276,178,PAPER)
para('首版的具体范围',541,420,246,13,19,bold=True)
para('受限纸包装结构、成型、开合，以及固定姿态直线取物计算。结果区分通过、不满足和未判定。可输出刀、折、胶SVG、材料数量及可编辑成果。',541,390,246,10.6,16.2,maxh=119)
columns(body(7),227,bottom=90,size=10.8)
end('来源：[8] 已发布版介绍源稿专业能力说明。图为接入机制；几何结果不等于材料保护性能或现实制造认证。')

# 10 — device assets plus operational definition.
begin('自定义设备，同时定义外观与工作方式','设备创作与模块复用','一张插图可以开始，一组姿态或一个模型可以深化；运行仍由明确的业务定义驱动。')
pic('assets/B60-105-zoom-minus960.jpg',M,240,345,207,crop=(650,400,1400,1050),border=False)
para('实际原生设备模型静态查看 · 媒体外观与业务定义分别管理。',M,229,345,9.3,13.5,MUTED)
ys=432
for title,desc in [('表现素材','插图、按顺序排列的图片、动图或3D模型。'),('工作定义','输入输出、工作阶段、工时与所需资源。'),('可设置内容','参数、预设数据标签与操作标签。'),('可复用交付','定义、素材、许可及最小运行示例。')]:
    text(title,419,ys,12,TEAL,True);para(desc,419,ys-14,383,10.5,16,maxh=33);ys-=56
columns(body(8),207,bottom=82,size=10.5)
end('外部建模工具可按项目规范提供资产；上传成功、视觉质量和业务接通是分别检查的内容。')

# 11 — Agent design and actual evidence scope distinguished once.
begin('Agent形成草案，人继续修改同一工程','Agent接入与人工接手','请求必须变成真实工程对象和平台操作，结果仍由运行给出。')
diagram('05_Agent工作方式',M,262,CW,180)
pars=body(9)[:3]
pars.append('现有材料包含Host与MCP消费、报告导出等调用证据。本文用机制图解释建厂工作方式，未把这些记录拼作“外部客户端一句话建厂”的完整桌面实录。')
columns(pars,244,bottom=95,size=10.7)
end('来源：[8] 发布版说明及本地实际Host工作流记录。当前所引记录范围不含外部客户端stdio、Native和HTTP整链实录。')

# 12 — task8 preserved as separate real SIL case.
begin('把设备数据、派工与回执接进工厂上下文','任务8的软件在环实例','模型解释计划；执行回执说明实际发生了什么。两者在工程中保持各自来源。')
diagram('06_软件在环链路',M,241,492,206)
pic('assets/Native-cloud-R354-original.png',553,282,249,154,crop=(260,100,1950,860))
para('实际云端执行观察画面\n固定来源与观察时点'.replace('\n','<br/>'),553,269,249,9.4,13.5,MUTED)
columns(body(10),221,bottom=89,size=10.6)
end('来源：[3] 研究任务8原文；[8] 两设备六件软件在环实例。物理设备接入与现场部署不在该实例的证明范围内。')

# 13 — honest numerical result, non-zero baselines avoided.
begin('比较结果之前，先解释改变了什么','结果与决策','两台软件设备处理六件工作；改变继续接新单的条件，观察完成过程。')
chartx=M+125;charty=302;chartw=365;maxv=100
for v in [0,25,50,75,100]:
    xx=chartx+v/maxv*chartw;line(xx,charty-5,xx,426,LINE,.5);text(str(v),xx-6,charty-22,8.5,MUTED)
for y,label,val,color in [(387,'正常实例',53.202,TEAL),(329,'停止A接新单',97.092,AMBER)]:
    para(label,M,y+16,113,11,16,bold=True);rect(chartx,y-5,val/maxv*chartw,25,color);text(f'{val:.3f} s',chartx+val/maxv*chartw+6,y+3,11,color,True)
text('Host过程时长／秒（轴从0开始）',chartx,charty-45,9.3,MUTED)
rect(608,297,194,142,PAPER);para('读图结论',623,424,164,13,19,bold=True)
para('停止一台设备继续接新单，会改变工作分配与完成过程。耗时变长是操作后果，不是优化收益。',623,391,164,10.4,16,maxh=82)
columns(body(11),228,bottom=88,size=10.5)
end('来源：[8] 任务8答复制作记录：normal_host_ms=53202；ban_host_ms=97092。独立实例，不代表物理产线节拍。')

# 14 — comparison in language, no unsupported invented scoring.
begin('贡献体现在工作链接续，而不靠功能计数','技术与组合价值','成熟工具已覆盖许多基础能力；本作品需要说明自己如何把任务完整接起来。')
rows=[['定义进入运行','设备、工艺、物流、资源与专业成果由运行机制实际消费。','修改输入后，能解释哪些结果随之变化。'],['过程支持决定','画布、逐件追踪、时间轴与评估指向同一业务过程。','能从等待返回具体条件，再决定怎样调整。'],['生成与交付接续','Agent草案可人工修改；专业成果和执行回执有明确接点。','接手者能继续编辑、运行和比较工程。']]
y=table(['接点','组合贡献','读者应怎样检查'],rows,[108,331,CW-439],M,445,font=10.6)
pars=body(12)
columns([pars[0],pars[-1]],y-23,bottom=116,size=11)
note('完整任务应展示：修改资源条件 → 解释等待变化 → 对应物品与时间轴 → 保存并由另一人重开。',M,113,CW)
end('成熟基线：[5] Plant Simulation Runtime产品说明；[6] Visual Components制造仿真产品页。')

# 15 — actionable contribution, source and governance as assets.
begin('开源的价值，是别人能够修改和继续使用','开源交付与参与','贡献可以从一台设备开始；维护要把定义、接口、许可和可用示例一起接住。')
diagram('07_贡献与接手',M,258,CW,184)
pars=body(13);pars[-1]='当前公开入口是[源码仓库](https://github.com/JunyeFu/superfactory-v2)和[v2.0.0发布页](https://github.com/JunyeFu/superfactory-v2/releases/tag/v2.0.0)。使用者按相应版本的运行说明与实际文件开始；代码许可和模型、图片的许可分别查看。'
columns(pars,239,bottom=87,size=10.6)
end('公开入口：[7] JunyeFu/superfactory-v2，v2.0.0。新介绍册为本地审阅版，尚未替换已发布材料。')

# 16 — adoption and economics, no invented TAM.
begin('从可复现的一份方案开始采用','应用与长期发展','先回答谁有任务、输入从哪里来、结果用于什么决定，再讨论服务和扩大覆盖。')
rows=[['工厂工程人员','工艺、设备、班次与物流条件','解释产能、等待及方案变化'],['系统集成商','可交接的设备与工艺定义','交付客户能继续修改的方案'],['教学与非专业使用者','适用模板、目标与待确认条件','理解约束，并由业务人员核对草案']]
y=table(['使用者','采用前准备','直接任务价值'],rows,[145,310,CW-455],M,445,font=10.6)
columns(body(14),y-22,bottom=87,size=10.5)
end('模板适配、模块开发、培训和集成支持是服务假设；本作品尚无客户、合同及收入数据支持市场规模结论。')

# 17 — real data / preserved work plus practical handoff instructions.
begin('把报告与工程一起交给下一位使用者','评估输出与接手','读报告可以理解一次决定；重开工程才能修改下一次决定。')
pic('assets/C12-calendar-label-fixed.jpg',M,266,470,179,crop=(280,965,1800,1270))
para('已保存运行的时间轴及等待说明局部。重开后读取原结果，不将历史回看当作新的活动执行。',M,252,470,9.4,14,MUTED,maxh=29)
rect(537,259,265,185,PAPER);para('已有报告输出',552,429,235,13,19,bold=True)
para('PDF / HTML / CSV / JSON',552,397,235,12,20,TEAL,bold=True)
para('任务8正常组与停用组均有四格式报告，每组22个文件，其中17张CSV。统计对应固定的实际报告来源。',552,360,235,10.6,16,maxh=84)
left=[('01  打开对应版本','从发布页取得代码、说明与样例。先沿样例运行，不把新行业模板当成已验证现场。'),('02  检查并修改输入','核对工艺、资源、班次与观察窗口；保留原方案，只改变这次需要评估的条件。')]
right=[('03  交付可解释的结果','交代输入与假设、结果来源、等待及适用范围。数据页的当前显示不等于全部筛选数据。'),('04  重开并接手','随工程交付相应定义、模块版本及许可允许的素材。接手者能修改，并保存自己的版本。')]
for x,group in [(M,left),(M+(CW+28)/2,right)]:
    y=211
    for title,desc in group:
        text(title,x,y,11.8,TEAL,True);y=para(desc,x,y-15,(CW-28)/2,10.8,16.2,maxh=64)-23
end('来源：[8] 已发布版报告与保存运行材料。发布页提供现有介绍PDF、可编辑源包及核心演示视频。')

# 18 — sources, edition note, public continuation.
begin('来源、版本与继续使用入口','参考与交付','文献事实、说明案例与实际结果分别归属；公开入口可在PDF内直接点击。')
refs=[
('01','广东省科技厅｜项目清单','母项目名称、编号2019B010120001及承担单位；相关页127，清单序号2050。','https://gdstc.gd.gov.cn/attachment/0/556/556949/4478318.pdf'),
('02','网络协同制造与工业互联网｜专项指南','省重点领域研发计划与设备互联、运维、调度研究背景。','https://cse.sysu.edu.cn/sites/default/files/2024-07/%E9%99%84%E4%BB%B61.%E2%80%9C%E7%BD%91%E7%BB%9C%E5%8D%8F%E5%90%8C%E5%88%B6%E9%80%A0%E4%B8%8E%E5%B7%A5%E4%B8%9A%E4%BA%92%E8%81%94%E7%BD%91%E2%80%9D%E9%87%8D%E7%82%B9%E4%B8%93%E9%A1%B9%E6%8C%87%E5%8D%97.pdf'),
('03','用户提供｜主要研究内容与任务8','任务8文字来自项目原文截图；作品延伸关系由用户确认。研究任务8与阶段评价表第8行分别处理。',None),
('04','西门子｜制造仿真与运营卓越','中文白皮书公开简介中的南京数控工厂案例。仅采用实际阅读范围，不将案例效益归本作品。','https://resources.sw.siemens.com/zh-CN/white-paper-manufacturing-simulation-operational-excellence/'),
('05','Siemens｜Plant Simulation Runtime','成熟工业仿真产品基线，含模型运行、可视化和结果分析相关能力。','https://static.sw.cdn.siemens.com/siemens-disw-assets/public/2RWVcx2SvqkXJc1GIQrwND/en-US/Siemens%20SW%20Plant%20Simulation%20Runtime%20Fact%20Sheet.pdf'),
('06','Visual Components｜Manufacturing Simulation','近邻方案与成熟制造仿真能力基线。','https://www.visualcomponents.com/products/manufacturing-simulation/'),
('07','超级工厂｜源码与v2.0.0发布','公开源码、对应运行资料、介绍文件及演示视频。','https://github.com/JunyeFu/superfactory-v2/releases/tag/v2.0.0'),
('08','本项目｜实际运行与技术答复材料','C12保存回看、两设备六件软件在环、四格式报告与MCP消费记录。源文件路径和图像归属随本册可编辑源保存。',None),
('09','制作方法｜Scientific Agent Skills','Kassis, Agarwal, He, Patel & Brueckner (2026). arXiv:2609.00065。用于表达与制作方法，不作为产品性能证据。','https://doi.org/10.48550/arXiv.2609.00065')]
for x,group in [(M,refs[:5]),(M+(CW+28)/2,refs[5:])]:
    y=444;w=(CW-28)/2
    for num,title,desc,url in group:
        label=f'[{num}] {title}'
        if url:label=f'[{num}] [{title}]({url})'
        y=para(label,x,y,w,10.1,14.6,bold=True)-5
        y=para(desc,x,y,w,9.4,14.1,MUTED)-13
note('版本说明：本册完成图文排版与阅读审校；素材沿用已发布版及具名实例。最终提交时再与候选软件、源码和视频统一冻结。',M,115,CW)
end('编制日期：2026-10-28。参考资料均按已有阅读记录使用；本册没有新增现场收益、客户或市场规模数字。')

C.save()
(ROOT/'页面清单.json').write_text(json.dumps(PAGES,ensure_ascii=False,indent=2),encoding='utf-8')
(ROOT/'素材使用.json').write_text(json.dumps(ASSETS,ensure_ascii=False,indent=2),encoding='utf-8')
(ROOT/'文字区域.json').write_text(json.dumps(BOXES,ensure_ascii=False,indent=2),encoding='utf-8')
print(json.dumps({'pdf':str(OUT),'pages':len(PAGES),'asset_uses':len(ASSETS),'text_boxes':len(BOXES)},ensure_ascii=False))
