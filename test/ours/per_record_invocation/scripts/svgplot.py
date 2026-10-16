"""極簡 SVG 折線圖（沿用 test/compare/results 既有圖表的樣式：白底、Noto Sans CJK TC、灰格線）。"""
import math

PALETTE = ["#2563eb", "#dc2626", "#16a34a", "#9333ea", "#ea580c", "#0891b2", "#475569"]
STYLE = ('text{font-family:"Noto Sans CJK TC","Noto Sans",sans-serif;fill:#172033}'
         '.tick{font-size:16px}.axis{font-size:18px}.note{font-size:14px;fill:#475569}'
         '.legend{font-size:16px}.title{font-size:27px;font-weight:700}.lab{font-size:13px;fill:#334155}')


def _fmt(v):
    if v >= 1e6:
        return f"{v/1e6:g}M"
    if v >= 1e3:
        return f"{v/1e3:g}K"
    return f"{v:g}"


def line_chart(path, title, note, xlabel, ylabel, series, xlog=True, ylog=False,
               xticks=None, yticks=None, ymin=None, ymax=None, labels=False, width=1400, height=800):
    """series: [(name, [(x, y), ...], dict(dash=bool, color=None))]"""
    L, R, T, B = 120, width - 330, 95, height - 95
    xs = [x for _, pts, _ in series for x, _ in pts]
    ys = [y for _, pts, _ in series for _, y in pts]
    x0, x1 = min(xs), max(xs)
    y0 = ymin if ymin is not None else (min(ys) if ylog else 0)
    y1 = ymax if ymax is not None else max(ys) * 1.1
    if xlog:
        fx = lambda x: L + (math.log(x) - math.log(x0)) / (math.log(x1) - math.log(x0) or 1) * (R - L)
    else:
        fx = lambda x: L + (x - x0) / ((x1 - x0) or 1) * (R - L)
    if ylog:
        fy = lambda y: B - (math.log(y) - math.log(y0)) / (math.log(y1) - math.log(y0)) * (B - T)
    else:
        fy = lambda y: B - (y - y0) / ((y1 - y0) or 1) * (B - T)
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
         '<rect width="100%" height="100%" fill="#ffffff"/>', f'<style>{STYLE}</style>',
         f'<text x="{width/2}" y="42" text-anchor="middle" class="title">{title}</text>',
         f'<text x="{width/2}" y="69" text-anchor="middle" class="note">{note}</text>']
    for xt in (xticks or sorted(set(xs))):
        X = fx(xt)
        o.append(f'<line x1="{X:.1f}" y1="{T}" x2="{X:.1f}" y2="{B}" stroke="#e2e8f0"/>')
        o.append(f'<text x="{X:.1f}" y="{B+30}" text-anchor="middle" class="tick">{_fmt(xt)}</text>')
    if yticks is None:
        if ylog:
            yticks = [v for e in range(-3, 9) for v in (10**e, 3 * 10**e) if y0 <= v <= y1]
        else:
            step = 10 ** math.floor(math.log10((y1 - y0) / 5 or 1))
            for m in (1, 2, 5, 10):
                if (y1 - y0) / (step * m) <= 8:
                    step *= m
                    break
            yticks = [y0 + i * step for i in range(int((y1 - y0) / step) + 1)]
    for yt in yticks:
        Y = fy(yt)
        o.append(f'<line x1="{L}" y1="{Y:.1f}" x2="{R}" y2="{Y:.1f}" stroke="#e2e8f0"/>')
        o.append(f'<text x="{L-12}" y="{Y+6:.1f}" text-anchor="end" class="tick">{_fmt(yt)}</text>')
    o.append(f'<line x1="{L}" y1="{B}" x2="{R}" y2="{B}" stroke="#334155" stroke-width="2"/>')
    o.append(f'<line x1="{L}" y1="{T}" x2="{L}" y2="{B}" stroke="#334155" stroke-width="2"/>')
    o.append(f'<text x="{(L+R)/2}" y="{B+70}" text-anchor="middle" class="axis">{xlabel}</text>')
    o.append(f'<text x="30" y="{(T+B)/2}" text-anchor="middle" transform="rotate(-90 30 {(T+B)/2})" class="axis">{ylabel}</text>')
    for i, (name, pts, opt) in enumerate(series):
        c = opt.get("color") or PALETTE[i % len(PALETTE)]
        dash = ' stroke-dasharray="8 6"' if opt.get("dash") else ""
        pts = sorted(pts)
        if len(pts) > 1:
            pl = " ".join(f"{fx(x):.1f},{fy(y):.1f}" for x, y in pts)
            o.append(f'<polyline points="{pl}" fill="none" stroke="{c}" stroke-width="3"{dash}/>')
        for x, y in pts:
            o.append(f'<circle cx="{fx(x):.1f}" cy="{fy(y):.1f}" r="5" fill="{c}"/>')
            if labels:
                o.append(f'<text x="{fx(x)+8:.1f}" y="{fy(y)-8:.1f}" class="lab">{y:,.0f}</text>')
        ly = T + 10 + i * 30
        o.append(f'<line x1="{R+25}" y1="{ly}" x2="{R+60}" y2="{ly}" stroke="{c}" stroke-width="3"{dash}/>')
        o.append(f'<circle cx="{R+42}" cy="{ly}" r="5" fill="{c}"/>')
        o.append(f'<text x="{R+70}" y="{ly+6}" class="legend">{name}</text>')
    o.append("</svg>")
    open(path, "w").write("\n".join(o))


def bar_chart(path, title, note, ylabel, groups, series_names, values, width=1400, height=800, unit=""):
    """groups: x 類別；values[s][g] = 數值（None 表示缺）。"""
    L, R, T, B = 120, width - 330, 95, height - 95
    vmax = max(v for row in values for v in row if v is not None) * 1.15
    fy = lambda y: B - y / vmax * (B - T)
    o = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
         '<rect width="100%" height="100%" fill="#ffffff"/>', f'<style>{STYLE}</style>',
         f'<text x="{width/2}" y="42" text-anchor="middle" class="title">{title}</text>',
         f'<text x="{width/2}" y="69" text-anchor="middle" class="note">{note}</text>']
    step = 10 ** math.floor(math.log10(vmax / 5))
    for m in (1, 2, 5, 10):
        if vmax / (step * m) <= 8:
            step *= m
            break
    t = 0
    while t <= vmax:
        Y = fy(t)
        o.append(f'<line x1="{L}" y1="{Y:.1f}" x2="{R}" y2="{Y:.1f}" stroke="#e2e8f0"/>')
        o.append(f'<text x="{L-12}" y="{Y+6:.1f}" text-anchor="end" class="tick">{_fmt(t)}</text>')
        t += step
    gw = (R - L) / len(groups)
    bw = gw * 0.8 / len(series_names)
    for gi, g in enumerate(groups):
        o.append(f'<text x="{L+gw*gi+gw/2:.1f}" y="{B+30}" text-anchor="middle" class="tick">{g}</text>')
        for si in range(len(series_names)):
            v = values[si][gi]
            if v is None:
                continue
            x = L + gw * gi + gw * 0.1 + bw * si
            c = PALETTE[si % len(PALETTE)]
            o.append(f'<rect x="{x:.1f}" y="{fy(v):.1f}" width="{bw-4:.1f}" height="{B-fy(v):.1f}" fill="{c}"/>')
            o.append(f'<text x="{x+bw/2-2:.1f}" y="{fy(v)-8:.1f}" text-anchor="middle" class="lab">{v:,.0f}{unit}</text>')
    o.append(f'<line x1="{L}" y1="{B}" x2="{R}" y2="{B}" stroke="#334155" stroke-width="2"/>')
    o.append(f'<line x1="{L}" y1="{T}" x2="{L}" y2="{B}" stroke="#334155" stroke-width="2"/>')
    o.append(f'<text x="30" y="{(T+B)/2}" text-anchor="middle" transform="rotate(-90 30 {(T+B)/2})" class="axis">{ylabel}</text>')
    for si, n in enumerate(series_names):
        ly = T + 10 + si * 30
        o.append(f'<rect x="{R+25}" y="{ly-8}" width="30" height="16" fill="{PALETTE[si % len(PALETTE)]}"/>')
        o.append(f'<text x="{R+65}" y="{ly+6}" class="legend">{n}</text>')
    o.append("</svg>")
    open(path, "w").write("\n".join(o))
