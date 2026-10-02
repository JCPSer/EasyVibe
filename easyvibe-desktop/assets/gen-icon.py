#!/usr/bin/env python3
"""EasyVibe 应用图标母版生成（1024x1024 RGBA，透明圆角）：
深靛→蓝对角渐变 squircle + 顶部液态高光 + 「架构分层」图形语言（三层横条经节点汇聚脊柱）。"""
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

S = 1024
R = 229  # 苹果图标圆角

# ---- 1. 对角渐变底色 ----
c_tl = np.array([0x31, 0x2E, 0x81], dtype=float)  # #312E81
c_mid = np.array([0x37, 0x30, 0xA3], dtype=float)  # #3730A3
c_br = np.array([0x25, 0x63, 0xEB], dtype=float)  # #2563EB
yy, xx = np.mgrid[0:S, 0:S]
t = (xx + yy) / (2 * S)
img = np.zeros((S, S, 3))
lo = t < 0.45
img[lo] = c_tl + (c_mid - c_tl) * (t[lo] / 0.45)[..., None]
img[~lo] = c_mid + (c_br - c_mid) * ((t[~lo] - 0.45) / 0.55)[..., None]

base = Image.fromarray(img.astype(np.uint8), 'RGB').convert('RGBA')

# ---- 2. squircle 蒙版 ----
mask = Image.new('L', (S, S), 0)
md = ImageDraw.Draw(mask)
md.rounded_rectangle([0, 0, S, S], radius=R, fill=255)
base.putalpha(mask)

# ---- 3. 顶部液态玻璃高光（垂直 alpha 渐变，仅顶部 45%）----
glass = Image.new('L', (S, S), 0)
ga = np.zeros((S, S), dtype=np.uint8)
rows = np.clip((1 - yy / (S * 0.45)) * 72, 0, 72).astype(np.uint8)
ga[:] = rows
glass = Image.fromarray(ga, 'L')
white = Image.new('RGBA', (S, S), (255, 255, 255, 0))
white.putalpha(Image.composite(glass, Image.new('L', (S, S), 0), mask))
base = Image.alpha_composite(base, white)

# ---- 4. 内晕光（青，中心 0.5,0.42）----
d2 = ((xx - S * 0.5) ** 2 + (yy - S * 0.42) ** 2) / (S * 0.55) ** 2
glow_a = np.clip(1 - d2, 0, 1) ** 1.5 * 90
glow = Image.new('RGBA', (S, S), (0x7D, 0xD3, 0xFC, 0))
glow.putalpha(Image.fromarray((glow_a * (np.array(mask) / 255)).astype(np.uint8), 'L'))
base = Image.alpha_composite(base, glow)

# ---- 5. 「架构分层」图形 ----
glyph = Image.new('RGBA', (S, S), (0, 0, 0, 0))
gd = ImageDraw.Draw(glyph)
LX, nodes_y = 352, [400, 512, 624]   # 脊柱 x 与三个节点 y
bar_x, bar_w = 436, [300, 238, 176]  # 三层横条
BH = 64

# 节点光晕（模糊青斑）
for ny in nodes_y:
    halo = Image.new('RGBA', (S, S), (0, 0, 0, 0))
    hd = ImageDraw.Draw(halo)
    hd.ellipse([LX - 26, ny - 26, LX + 26, ny + 26], fill=(0x22, 0xD3, 0xEE, 120))
    glyph = Image.alpha_composite(glyph, halo.filter(ImageFilter.GaussianBlur(14)))
    gd = ImageDraw.Draw(glyph)

# 连接线（脊柱 + 横线）
gd.line([(LX, nodes_y[0]), (LX, nodes_y[2])], fill=(0xBA, 0xE6, 0xFD, 215), width=14)
for ny in nodes_y:
    gd.line([(LX, ny), (bar_x, ny)], fill=(0xBA, 0xE6, 0xFD, 215), width=14)

# 三层横条（白，上宽下窄）
for w, ny in zip(bar_w, nodes_y):
    gd.rounded_rectangle([bar_x, ny - BH // 2, bar_x + w, ny + BH // 2], radius=BH // 2, fill=(0xF8, 0xFA, 0xFC, 255))

# 节点（青芯）
for ny in nodes_y:
    gd.ellipse([LX - 17, ny - 17, LX + 17, ny + 17], fill=(0xA5, 0xF3, 0xFC, 255))

# 图形整体轻投影（提升层次感）
shadow = glyph.filter(ImageFilter.GaussianBlur(10))
shadow_a = shadow.split()[3].point(lambda a: a * 0.35)
shadow.putalpha(shadow_a)
base = Image.alpha_composite(base, shadow)
base = Image.alpha_composite(base, glyph)

base.save('/Users/liyuhang/Documents/EasyVibe/easyvibe-desktop/assets/app-icon-1024.png')
print('saved 1024x1024 RGBA')
