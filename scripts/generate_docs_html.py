import os, re, html, json

docs_dir = "/home/user/git_test/projects/tru/repos/truswap/docs"

chapters = [
    {"id": "readme", "file": "README.md", "title": "Overview & Architecture", "icon": "⚡", "num": "00"},
    {"id": "01_introduction", "file": "01_introduction.md", "title": "1. Introduction & Vision", "icon": "🌐", "num": "01"},
    {"id": "02_quickstart_desktop", "file": "02_quickstart_desktop.md", "title": "2. Desktop Quick Start", "icon": "💻", "num": "02"},
    {"id": "03_quickstart_android", "file": "03_quickstart_android.md", "title": "3. Android Mobile Setup", "icon": "📱", "num": "03"},
    {"id": "04_vault_and_security", "file": "04_vault_and_security.md", "title": "4. Vault & Key Security", "icon": "🔐", "num": "04"},
    {"id": "05_atomic_swap_protocol", "file": "05_atomic_swap_protocol.md", "title": "5. Atomic Swap Protocol", "icon": "🔄", "num": "05"},
    {"id": "06_tru_blockchain_engine", "file": "06_tru_blockchain_engine.md", "title": "6. TRU Blockchain Engine", "icon": "⛏️", "num": "06"},
    {"id": "07_base_layer2_engine", "file": "07_base_layer2_engine.md", "title": "7. Base Layer-2 Engine", "icon": "🔷", "num": "07"},
    {"id": "08_p2p_orderbook_mesh", "file": "08_p2p_orderbook_mesh.md", "title": "8. P2P Orderbook Mesh", "icon": "🕸️", "num": "08"},
    {"id": "09_mobile_ndk_hardware", "file": "09_mobile_ndk_hardware.md", "title": "9. NDK Camera & Hardware", "icon": "📷", "num": "09"},
    {"id": "10_security_model_and_threat_analysis", "file": "10_security_model_and_threat_analysis.md", "title": "10. Security & Threat Model", "icon": "🛡️", "num": "10"},
    {"id": "11_troubleshooting_and_faq", "file": "11_troubleshooting_and_faq.md", "title": "11. Troubleshooting & FAQ", "icon": "❓", "num": "11"},
]

def format_inline(s):
    s = html.escape(s)
    # inline code
    s = re.sub(r"`([^`]+)`", r"<code>\1</code>", s)
    # bold
    s = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", s)
    # italic
    s = re.sub(r"\*([^*]+)\*", r"<em>\1</em>", s)
    # links: rewrite internal .md links to hash links
    def link_repl(m):
        txt, target = m.group(1), m.group(2)
        if target.endswith(".md"):
            t_id = target[:-3]
            if t_id == "README":
                t_id = "readme"
            return f'<a href="#{t_id}" class="internal-link" onclick="loadChapter(\'{t_id}\')">{txt}</a>'
        return f'<a href="{target}" target="_blank" rel="noopener">{txt}</a>'
    s = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", link_repl, s)
    return s

def md_to_html(text):
    lines = text.split("\n")
    out = []
    in_code = False
    code_lang = ""
    code_lines = []
    in_table = False
    table_rows = []
    in_list = False
    list_type = "ul"
    in_blockquote = False
    quote_lines = []

    def flush_list():
        nonlocal in_list
        if in_list:
            out.append(f"</{list_type}>")
            in_list = False

    def flush_table():
        nonlocal in_table, table_rows
        if in_table and table_rows:
            html_t = ["<div class=\"table-responsive\"><table class=\"doc-table\">"]
            header = table_rows[0]
            html_t.append("<thead><tr>")
            for cell in header:
                html_t.append(f"<th>{cell}</th>")
            html_t.append("</tr></thead>")
            if len(table_rows) > 1:
                html_t.append("<tbody>")
                for row in table_rows[1:]:
                    html_t.append("<tr>")
                    for cell in row:
                        html_t.append(f"<td>{cell}</td>")
                    html_t.append("</tr>")
                html_t.append("</tbody>")
            html_t.append("</table></div>")
            out.append("\n".join(html_t))
            in_table = False
            table_rows = []

    def flush_quote():
        nonlocal in_blockquote, quote_lines
        if in_blockquote:
            raw = "\n".join(quote_lines)
            alert_match = re.match(r"^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(.*)", raw, re.DOTALL)
            if alert_match:
                kind = alert_match.group(1).lower()
                body = alert_match.group(2).strip()
                out.append(f"<div class=\"doc-alert doc-alert-{kind}\"><div class=\"alert-title\">{kind.upper()}</div><div class=\"alert-body\">{format_inline(body)}</div></div>")
            else:
                out.append(f"<blockquote class=\"doc-quote\">{format_inline(raw)}</blockquote>")
            in_blockquote = False
            quote_lines = []

    for line in lines:
        stripped = line.strip()

        # Allow raw HTML blocks (e.g. custom responsive diagram components)
        if not in_code and stripped.startswith("<") and not re.match(r"^<https?://", stripped) and not re.match(r"^<[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+>$", stripped):
            flush_list()
            flush_table()
            flush_quote()
            out.append(line)
            continue

        if stripped.startswith("```"):
            if in_code:
                code_text = "\n".join(code_lines)
                is_box_diagram = any(c in code_text for c in ["┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼", "│"])
                code_content = html.escape(code_text)
                if is_box_diagram or code_lang.lower() == "diagram":
                    out.append(f"<div class=\"ascii-diagram-wrapper\"><div class=\"ascii-diagram-header\"><span class=\"ascii-diagram-tag\">📐 SCHEMATIC DIAGRAM</span><span class=\"ascii-diagram-scroll-hint\">↔ Scroll diagram</span></div><pre class=\"ascii-diagram-pre\"><code class=\"language-diagram\">{code_content}</code></pre></div>")
                else:
                    out.append(f"<div class=\"code-block-wrapper\"><div class=\"code-header\"><span class=\"code-lang\">{code_lang.upper()}</span><button class=\"copy-btn\" onclick=\"copyCode(this)\" title=\"Copy code to clipboard\">📋 Copy</button></div><pre><code class=\"language-{code_lang}\">{code_content}</code></pre></div>")
                in_code = False
                code_lines = []
                code_lang = ""
            else:
                flush_list()
                flush_table()
                flush_quote()
                in_code = True
                code_lang = stripped[3:].strip() or "text"
            continue

        if in_code:
            code_lines.append(line)
            continue

        if stripped.startswith(">"):
            flush_list()
            flush_table()
            in_blockquote = True
            quote_lines.append(stripped[1:].strip())
            continue
        elif in_blockquote:
            flush_quote()

        if "|" in line and (line.startswith("|") or stripped.endswith("|")):
            flush_list()
            parts = [format_inline(c.strip()) for c in line.strip().strip("|").split("|")]
            if parts and all(re.match(r"^:?-+:?$", p) for p in parts):
                continue
            in_table = True
            table_rows.append(parts)
            continue
        elif in_table:
            flush_table()

        if stripped.startswith("---") or stripped.startswith("***"):
            flush_list()
            out.append("<hr class=\"doc-divider\">")
            continue

        h_match = re.match(r"^(#{1,6})\s+(.*)$", stripped)
        if h_match:
            flush_list()
            level = len(h_match.group(1))
            htext = h_match.group(2)
            hid = re.sub(r"[^a-z0-9]+", "-", htext.lower()).strip("-")
            out.append(f"<h{level} id=\"{hid}\" class=\"doc-heading doc-h{level}\">{format_inline(htext)}<a class=\"anchor-link\" href=\"#{hid}\" title=\"Direct Link\">#</a></h{level}>")
            continue

        ul_match = re.match(r"^[-*]\s+(.*)$", stripped)
        if ul_match:
            if not in_list or list_type != "ul":
                flush_list()
                in_list = True
                list_type = "ul"
                out.append("<ul class=\"doc-list\">")
            out.append(f"<li>{format_inline(ul_match.group(1))}</li>")
            continue

        ol_match = re.match(r"^\d+\.\s+(.*)$", stripped)
        if ol_match:
            if not in_list or list_type != "ol":
                flush_list()
                in_list = True
                list_type = "ol"
                out.append("<ol class=\"doc-list\">")
            out.append(f"<li>{format_inline(ol_match.group(1))}</li>")
            continue

        if in_list:
            flush_list()

        if not stripped:
            continue

        out.append(f"<p class=\"doc-para\">{format_inline(stripped)}</p>")

    flush_list()
    flush_table()
    flush_quote()
    return "\n".join(out)

chapters_data = []
for ch in chapters:
    fpath = os.path.join(docs_dir, ch["file"])
    with open(fpath, "r", encoding="utf-8") as fp:
        raw = fp.read()
    word_count = len(raw.split())
    read_mins = max(1, round(word_count / 200))
    chapters_data.append({
        "id": ch["id"],
        "num": ch["num"],
        "file": ch["file"],
        "title": ch["title"],
        "icon": ch["icon"],
        "words": word_count,
        "read_time": f"{read_mins} min read",
        "html": md_to_html(raw),
        "raw_text": re.sub(r"[#*_`|>-]", " ", raw)
    })

chapters_json = json.dumps(chapters_data)

html_template = f'''<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>The TRUSWAP Book — Comprehensive Guide & Architecture</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700;800&family=Outfit:wght@300;400;500;600;700;800&display=swap" rel="stylesheet">
  <style>
    :root {{
      --bg-dark: #07090e;
      --bg-sidebar: #0b0f19;
      --bg-card: #111726;
      --bg-card-hover: #161e30;
      --bg-code: #06080d;
      --border-color: rgba(0, 240, 255, 0.18);
      --border-glow: rgba(0, 240, 255, 0.4);
      --accent-cyan: #00f0ff;
      --accent-cyan-rgb: 0, 240, 255;
      --accent-gold: #f59e0b;
      --accent-green: #10b981;
      --accent-red: #ef4444;
      --text-main: #f1f5f9;
      --text-muted: #94a3b8;
      --text-dim: #64748b;
      --sidebar-width: 320px;
      --header-height: 64px;
    }}

    * {{
      box-sizing: border-box;
      margin: 0;
      padding: 0;
    }}

    html, body {{
      background-color: var(--bg-dark);
      color: var(--text-main);
      font-family: 'Outfit', -apple-system, BlinkMacSystemFont, sans-serif;
      font-size: 16px;
      line-height: 1.65;
      height: 100%;
      overflow: hidden;
    }}

    /* Custom Scrollbar */
    ::-webkit-scrollbar {{
      width: 7px;
      height: 7px;
    }}
    ::-webkit-scrollbar-track {{
      background: rgba(11, 15, 25, 0.7);
    }}
    ::-webkit-scrollbar-thumb {{
      background: rgba(0, 240, 255, 0.25);
      border-radius: 4px;
    }}
    ::-webkit-scrollbar-thumb:hover {{
      background: rgba(0, 240, 255, 0.5);
    }}

    /* App Shell */
    .app-layout {{
      display: flex;
      flex-direction: column;
      height: 100vh;
      overflow: hidden;
    }}

    /* Mecha Cyberpunk Header */
    header.book-header {{
      height: var(--header-height);
      background: linear-gradient(180deg, #131a29 0%, #0c111c 100%);
      border-bottom: 1px solid var(--border-color);
      box-shadow: 0 4px 20px rgba(0, 0, 0, 0.6), inset 0 1px 0 rgba(255, 255, 255, 0.08);
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 24px;
      z-index: 100;
      position: relative;
    }}

    .header-left {{
      display: flex;
      align-items: center;
      gap: 16px;
      flex-shrink: 0;
    }}

    .mobile-menu-btn {{
      display: none;
      background: rgba(0, 240, 255, 0.1);
      border: 1px solid var(--border-color);
      color: var(--accent-cyan);
      font-size: 1.25rem;
      border-radius: 8px;
      width: 38px;
      height: 38px;
      cursor: pointer;
      align-items: center;
      justify-content: center;
      transition: all 0.2s ease;
      flex-shrink: 0;
    }}

    .mobile-menu-btn:hover {{
      background: rgba(0, 240, 255, 0.2);
      border-color: var(--accent-cyan);
    }}

    .brand-group {{
      display: flex;
      align-items: center;
      gap: 10px;
      text-decoration: none;
      cursor: pointer;
      flex-shrink: 0;
    }}

    .brand-logo-gem {{
      width: 28px;
      height: 28px;
      background: linear-gradient(135deg, #00f0ff 0%, #3b82f6 100%);
      clip-path: polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%);
      box-shadow: 0 0 12px rgba(0, 240, 255, 0.6);
      flex-shrink: 0;
    }}

    .brand-title-wrap {{
      display: flex;
      align-items: center;
      gap: 8px;
      white-space: nowrap;
    }}

    .brand-title {{
      font-family: 'JetBrains Mono', monospace;
      font-weight: 800;
      font-size: 1.15rem;
      letter-spacing: 0.05em;
      color: #fff;
      text-shadow: 0 0 10px rgba(0, 240, 255, 0.5);
      white-space: nowrap;
    }}

    .brand-badge {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.7rem;
      font-weight: 700;
      background: rgba(0, 240, 255, 0.15);
      border: 1px solid var(--border-color);
      color: var(--accent-cyan);
      padding: 2px 7px;
      border-radius: 4px;
      letter-spacing: 0.05em;
      white-space: nowrap;
      display: inline-block;
    }}

    .header-search-wrap {{
      flex: 1;
      max-width: 440px;
      margin: 0 24px;
      position: relative;
    }}

    .search-input {{
      width: 100%;
      background: #080c14;
      border: 1px solid rgba(0, 240, 255, 0.22);
      border-radius: 8px;
      padding: 9px 38px 9px 38px;
      font-family: 'Outfit', sans-serif;
      font-size: 0.88rem;
      color: #fff;
      transition: all 0.2s ease;
    }}

    .search-input:focus {{
      outline: none;
      border-color: var(--accent-cyan);
      box-shadow: 0 0 12px rgba(0, 240, 255, 0.3);
    }}

    .search-icon {{
      position: absolute;
      left: 12px;
      top: 50%;
      transform: translateY(-50%);
      color: var(--text-dim);
      font-size: 0.9rem;
      pointer-events: none;
    }}

    .search-shortcut {{
      position: absolute;
      right: 10px;
      top: 50%;
      transform: translateY(-50%);
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.7rem;
      background: rgba(255, 255, 255, 0.08);
      color: var(--text-dim);
      padding: 2px 6px;
      border-radius: 4px;
      pointer-events: none;
    }}

    .search-results-dropdown {{
      display: none;
      position: absolute;
      top: calc(100% + 6px);
      left: 0;
      right: 0;
      background: #0d121f;
      border: 1px solid var(--border-color);
      border-radius: 8px;
      box-shadow: 0 12px 30px rgba(0, 0, 0, 0.8);
      max-height: 380px;
      overflow-y: auto;
      z-index: 200;
    }}

    .search-result-item {{
      padding: 10px 14px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.05);
      cursor: pointer;
      transition: background 0.15s ease;
    }}

    .search-result-item:hover {{
      background: rgba(0, 240, 255, 0.1);
    }}

    .search-result-title {{
      font-weight: 600;
      font-size: 0.88rem;
      color: var(--accent-cyan);
      margin-bottom: 4px;
    }}

    .search-result-snippet {{
      font-size: 0.78rem;
      color: var(--text-muted);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}

    .header-links {{
      display: flex;
      align-items: center;
      gap: 12px;
      flex-shrink: 0;
    }}

    .header-nav-link {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.82rem;
      color: var(--text-muted);
      text-decoration: none;
      padding: 6px 12px;
      border-radius: 6px;
      background: rgba(255, 255, 255, 0.03);
      border: 1px solid rgba(255, 255, 255, 0.08);
      transition: all 0.2s ease;
      white-space: nowrap;
    }}

    .header-nav-link:hover {{
      color: #fff;
      border-color: var(--accent-cyan);
      background: rgba(0, 240, 255, 0.08);
      box-shadow: 0 0 10px rgba(0, 240, 255, 0.2);
    }}

    .mobile-search-btn {{
      display: none;
    }}

    .mobile-search-bar {{
      display: none;
    }}

    .nav-text-short {{
      display: none;
    }}

    .nav-text-full {{
      display: inline;
    }}

    /* Sidebar Backdrop & Mobile Drawer */
    .sidebar-backdrop {{
      display: none;
      position: fixed;
      top: 0;
      left: 0;
      right: 0;
      bottom: 0;
      background: rgba(0, 0, 0, 0.7);
      backdrop-filter: blur(3px);
      z-index: 140;
    }}

    .sidebar-backdrop.open {{
      display: block;
    }}

    .sidebar-footer {{
      display: none;
      padding: 16px 12px;
      margin-top: auto;
      border-top: 1px solid var(--border-color);
      background: rgba(6, 8, 14, 0.6);
      flex-direction: column;
      gap: 8px;
    }}

    .sidebar-footer-link {{
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 9px 12px;
      border-radius: 6px;
      background: rgba(255, 255, 255, 0.03);
      border: 1px solid rgba(255, 255, 255, 0.08);
      color: var(--text-muted);
      text-decoration: none;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.82rem;
      transition: all 0.2s ease;
    }}

    .sidebar-footer-link:hover {{
      color: #fff;
      border-color: var(--accent-cyan);
      background: rgba(0, 240, 255, 0.08);
    }}

    /* Main Container */
    .book-body {{
      display: flex;
      flex: 1;
      height: calc(100vh - var(--header-height));
      overflow: hidden;
      position: relative;
    }}

    /* Sidebar Table of Contents */
    aside.book-sidebar {{
      width: var(--sidebar-width);
      min-width: var(--sidebar-width);
      background: var(--bg-sidebar);
      border-right: 1px solid var(--border-color);
      display: flex;
      flex-direction: column;
      overflow-y: auto;
      height: 100%;
      user-select: none;
    }}

    .sidebar-section-title {{
      padding: 16px 20px 8px 20px;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      font-weight: 700;
      color: var(--text-dim);
      letter-spacing: 0.1em;
      text-transform: uppercase;
    }}

    .chapter-list {{
      list-style: none;
      padding: 0 10px 30px 10px;
    }}

    .chapter-item {{
      margin-bottom: 2px;
    }}

    .chapter-link {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      padding: 9px 12px;
      border-radius: 7px;
      color: var(--text-muted);
      text-decoration: none;
      font-size: 0.88rem;
      font-weight: 500;
      transition: all 0.15s ease;
      cursor: pointer;
      position: relative;
    }}

    .chapter-link:hover {{
      color: #fff;
      background: rgba(0, 240, 255, 0.06);
    }}

    .chapter-link.active {{
      color: #fff;
      font-weight: 600;
      background: linear-gradient(90deg, rgba(0, 240, 255, 0.16) 0%, rgba(0, 240, 255, 0.04) 100%);
      border-left: 3px solid var(--accent-cyan);
    }}

    .chapter-link-left {{
      display: flex;
      align-items: center;
      gap: 10px;
      overflow: hidden;
    }}

    .chapter-icon {{
      font-size: 1rem;
      flex-shrink: 0;
    }}

    .chapter-title-text {{
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}

    .chapter-num {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      color: var(--text-dim);
      background: rgba(255, 255, 255, 0.04);
      padding: 2px 6px;
      border-radius: 4px;
      flex-shrink: 0;
    }}

    /* Main Reader Area */
    main.book-content-pane {{
      flex: 1;
      overflow-y: auto;
      height: 100%;
      padding: 36px 48px 80px 48px;
      position: relative;
      scroll-behavior: smooth;
    }}

    .content-container {{
      max-width: 920px;
      margin: 0 auto;
    }}

    /* Top Breadcrumbs & Meta */
    .chapter-header-meta {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 24px;
      padding-bottom: 14px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    }}

    .breadcrumbs {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.82rem;
      color: var(--text-dim);
    }}

    .breadcrumbs span.current {{
      color: var(--accent-cyan);
    }}

    .read-time-badge {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.78rem;
      color: var(--accent-gold);
      background: rgba(245, 158, 11, 0.12);
      border: 1px solid rgba(245, 158, 11, 0.3);
      padding: 3px 10px;
      border-radius: 12px;
    }}

    /* Typography inside reader */
    .doc-h1 {{
      font-size: 2.2rem;
      font-weight: 800;
      color: #fff;
      margin-bottom: 20px;
      line-height: 1.25;
      letter-spacing: -0.02em;
      position: relative;
    }}

    .doc-h2 {{
      font-size: 1.45rem;
      font-weight: 700;
      color: #f8fafc;
      margin-top: 40px;
      margin-bottom: 16px;
      letter-spacing: -0.01em;
      border-bottom: 1px solid rgba(0, 240, 255, 0.15);
      padding-bottom: 8px;
      position: relative;
    }}

    .doc-h3 {{
      font-size: 1.18rem;
      font-weight: 600;
      color: var(--accent-cyan);
      margin-top: 26px;
      margin-bottom: 12px;
      position: relative;
    }}

    .anchor-link {{
      opacity: 0;
      margin-left: 8px;
      color: var(--text-dim);
      text-decoration: none;
      font-size: 0.85em;
      transition: opacity 0.15s ease;
    }}

    .doc-heading:hover .anchor-link {{
      opacity: 0.8;
    }}

    .doc-para {{
      margin-bottom: 16px;
      color: #cbd5e1;
      font-size: 1rem;
    }}

    .doc-list {{
      margin: 14px 0 20px 24px;
      color: #cbd5e1;
    }}

    .doc-list li {{
      margin-bottom: 8px;
    }}

    .doc-divider {{
      border: none;
      border-top: 1px solid rgba(0, 240, 255, 0.16);
      margin: 36px 0;
    }}

    /* Code Blocks */
    .code-block-wrapper {{
      margin: 22px 0;
      border-radius: 10px;
      background: var(--bg-code);
      border: 1px solid rgba(0, 240, 255, 0.22);
      box-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
      overflow: hidden;
    }}

    .code-header {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 8px 14px;
      background: #0d121c;
      border-bottom: 1px solid rgba(0, 240, 255, 0.12);
    }}

    .code-lang {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      font-weight: 700;
      color: var(--accent-cyan);
      letter-spacing: 0.08em;
    }}

    .copy-btn {{
      background: rgba(255, 255, 255, 0.05);
      border: 1px solid rgba(255, 255, 255, 0.15);
      color: var(--text-muted);
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      padding: 3px 9px;
      border-radius: 5px;
      cursor: pointer;
      transition: all 0.15s ease;
    }}

    .copy-btn:hover {{
      color: #fff;
      border-color: var(--accent-cyan);
      background: rgba(0, 240, 255, 0.15);
    }}

    pre {{
      padding: 16px;
      overflow-x: auto;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.88rem;
      line-height: 1.45;
      color: #e2e8f0;
      white-space: pre;
      word-wrap: normal;
    }}

    code {{
      font-family: 'JetBrains Mono', monospace;
      white-space: inherit;
    }}

    p code, li code {{
      background: rgba(0, 240, 255, 0.08);
      border: 1px solid rgba(0, 240, 255, 0.2);
      color: #38bdf8;
      padding: 2px 6px;
      border-radius: 4px;
      font-size: 0.88em;
      white-space: normal;
    }}

    /* Responsive Cyberpunk Architecture & Flow Diagrams */
    .cyber-diagram-box {{
      margin: 24px 0;
      background: linear-gradient(180deg, #0e1526 0%, #080c16 100%);
      border: 1px solid var(--border-color);
      border-radius: 12px;
      padding: 20px;
      box-shadow: 0 8px 30px rgba(0, 0, 0, 0.7), inset 0 1px 0 rgba(0, 240, 255, 0.15);
      position: relative;
      overflow: hidden;
    }}

    .diagram-title-bar {{
      display: flex;
      align-items: center;
      gap: 10px;
      margin-bottom: 18px;
      padding-bottom: 12px;
      border-bottom: 1px solid rgba(0, 240, 255, 0.12);
    }}

    .diagram-tag {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.68rem;
      font-weight: 700;
      background: rgba(0, 240, 255, 0.15);
      color: var(--accent-cyan);
      border: 1px solid rgba(0, 240, 255, 0.3);
      padding: 2px 7px;
      border-radius: 4px;
      letter-spacing: 0.08em;
    }}

    .diagram-heading {{
      font-weight: 700;
      font-size: 1rem;
      color: #fff;
      letter-spacing: 0.02em;
    }}

    .diagram-grid-2 {{
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 16px;
    }}

    .diagram-node {{
      background: #090e1a;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 8px;
      padding: 14px 16px;
      transition: border-color 0.2s ease;
    }}

    .diagram-node.node-tru {{
      border-left: 3px solid var(--accent-gold);
    }}

    .diagram-node.node-base {{
      border-left: 3px solid #3b82f6;
    }}

    .diagram-node.node-app {{
      border-left: 3px solid var(--accent-cyan);
      background: linear-gradient(180deg, #0d1526 0%, #080c14 100%);
    }}

    .node-badge {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.68rem;
      font-weight: 700;
      letter-spacing: 0.06em;
      margin-bottom: 4px;
    }}

    .node-tru .node-badge {{ color: var(--accent-gold); }}
    .node-base .node-badge {{ color: #60a5fa; }}
    .node-app .node-badge {{ color: var(--accent-cyan); }}

    .node-title {{
      font-size: 1rem;
      font-weight: 700;
      color: #fff;
      margin-bottom: 8px;
    }}

    .node-list {{
      list-style: none;
      padding: 0;
      margin: 0;
      display: flex;
      flex-direction: column;
      gap: 6px;
      font-size: 0.85rem;
      color: #cbd5e1;
    }}

    .node-list li {{
      display: flex;
      align-items: center;
      gap: 6px;
    }}

    .node-list li::before {{
      content: "•";
      color: var(--accent-cyan);
      font-weight: bold;
    }}

    .diagram-connector {{
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      margin: 12px 0;
      color: var(--accent-cyan);
    }}

    .connector-line {{
      width: 2px;
      height: 16px;
      background: linear-gradient(180deg, rgba(0, 240, 255, 0.4) 0%, rgba(0, 240, 255, 0.8) 100%);
    }}

    .connector-arrow {{
      font-size: 0.75rem;
      line-height: 1;
      color: var(--accent-cyan);
      text-shadow: 0 0 8px rgba(0, 240, 255, 0.8);
    }}

    .node-tags-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 8px;
      margin-top: 10px;
    }}

    .feature-tag {{
      background: rgba(0, 240, 255, 0.05);
      border: 1px solid rgba(0, 240, 255, 0.15);
      padding: 6px 10px;
      border-radius: 6px;
      font-size: 0.8rem;
      color: #e2e8f0;
      display: flex;
      align-items: center;
      gap: 6px;
    }}

    .feature-tag::before {{
      content: "▸";
      color: var(--accent-cyan);
    }}

    /* Atomic Swap Outcome Cards */
    .outcome-grid {{
      display: flex;
      align-items: stretch;
      gap: 16px;
      position: relative;
    }}

    .outcome-card {{
      flex: 1;
      border-radius: 10px;
      padding: 16px;
      background: #090e1a;
      border: 1px solid rgba(255, 255, 255, 0.08);
      display: flex;
      flex-direction: column;
      gap: 10px;
    }}

    .outcome-card.success {{
      border-left: 4px solid var(--accent-green);
      background: linear-gradient(180deg, rgba(16, 185, 129, 0.08) 0%, #080c14 100%);
    }}

    .outcome-card.refund {{
      border-left: 4px solid var(--accent-gold);
      background: linear-gradient(180deg, rgba(245, 158, 11, 0.08) 0%, #080c14 100%);
    }}

    .outcome-header {{
      display: flex;
      align-items: center;
      gap: 8px;
    }}

    .outcome-icon {{
      font-size: 1.1rem;
    }}

    .outcome-title {{
      font-weight: 700;
      font-size: 0.95rem;
      color: #fff;
    }}

    .outcome-condition {{
      font-size: 0.8rem;
      color: var(--text-dim);
      font-family: 'JetBrains Mono', monospace;
      padding-bottom: 8px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    }}

    .outcome-points {{
      list-style: none;
      padding: 0;
      margin: 0;
      display: flex;
      flex-direction: column;
      gap: 6px;
      font-size: 0.84rem;
      color: #cbd5e1;
    }}

    .outcome-points li strong {{
      color: #fff;
    }}

    .outcome-divider-badge {{
      display: flex;
      align-items: center;
      justify-content: center;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      font-weight: 800;
      color: var(--text-dim);
      background: #0d1322;
      border: 1px solid rgba(255, 255, 255, 0.1);
      padding: 4px 10px;
      border-radius: 12px;
      align-self: center;
      flex-shrink: 0;
    }}

    /* UI Tour Blueprint Card */
    .ui-tour-diagram .ui-mockup-frame {{
      display: flex;
      flex-direction: column;
      gap: 10px;
      background: #05070c;
      border: 1px solid rgba(0, 240, 255, 0.2);
      border-radius: 8px;
      padding: 12px;
    }}

    .ui-mockup-top {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 8px 12px;
      background: #0c1220;
      border-radius: 6px;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.76rem;
      color: var(--accent-cyan);
    }}

    .ui-mockup-vaults {{
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 10px;
    }}

    .mockup-card {{
      background: #090e1a;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 6px;
      padding: 10px 12px;
      font-size: 0.82rem;
    }}

    .mockup-card.tru-card {{ border-top: 2px solid var(--accent-gold); }}
    .mockup-card.base-card {{ border-top: 2px solid #3b82f6; }}

    .mockup-card .card-title {{
      font-weight: 700;
      margin-bottom: 4px;
      color: #fff;
    }}

    .mockup-card .card-detail {{
      color: #94a3b8;
      font-size: 0.78rem;
      margin-bottom: 6px;
    }}

    .mockup-card .card-pills {{
      display: flex;
      flex-wrap: wrap;
      gap: 4px;
    }}

    .mockup-card .pill {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.68rem;
      background: rgba(255, 255, 255, 0.05);
      border: 1px solid rgba(255, 255, 255, 0.1);
      padding: 2px 5px;
      border-radius: 4px;
      color: var(--text-dim);
    }}

    .ui-mockup-section {{
      background: #090e1a;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 6px;
      padding: 10px 12px;
    }}

    .ui-mockup-section .section-title {{
      font-weight: 700;
      font-size: 0.82rem;
      color: var(--accent-cyan);
      margin-bottom: 6px;
    }}

    .mockup-form-row {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      font-size: 0.78rem;
      color: #94a3b8;
      font-family: 'JetBrains Mono', monospace;
    }}

    .mockup-table-preview {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.74rem;
      color: #94a3b8;
      display: flex;
      flex-direction: column;
      gap: 4px;
    }}

    .mockup-row {{
      display: flex;
      justify-content: space-between;
      padding: 4px 8px;
      background: rgba(0, 240, 255, 0.04);
      border-radius: 4px;
      color: #e2e8f0;
    }}

    .mockup-row .action-btn {{
      color: var(--accent-cyan);
      font-weight: bold;
    }}

    /* Flow Steps Architecture Cards */
    .flow-steps {{
      display: flex;
      flex-direction: column;
      gap: 8px;
    }}

    .flow-step {{
      display: flex;
      align-items: flex-start;
      gap: 12px;
      background: #090e1a;
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 8px;
      padding: 12px 14px;
    }}

    .flow-step.node-tru {{
      border-left: 3px solid var(--accent-gold);
    }}

    .flow-step.node-base {{
      border-left: 3px solid #3b82f6;
    }}

    .flow-step-num {{
      width: 24px;
      height: 24px;
      border-radius: 50%;
      background: rgba(0, 240, 255, 0.15);
      border: 1px solid rgba(0, 240, 255, 0.4);
      color: var(--accent-cyan);
      font-family: 'JetBrains Mono', monospace;
      font-weight: 700;
      font-size: 0.75rem;
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;
      margin-top: 1px;
    }}

    .flow-step-content {{
      flex: 1;
    }}

    .flow-step-title {{
      font-weight: 700;
      font-size: 0.95rem;
      color: #fff;
      margin-bottom: 3px;
    }}

    .flow-step-desc {{
      font-size: 0.84rem;
      color: #94a3b8;
      line-height: 1.45;
    }}

    .flow-step-desc code {{
      font-family: 'JetBrains Mono', monospace;
      color: var(--accent-cyan);
      background: rgba(0, 240, 255, 0.08);
      padding: 1px 4px;
      border-radius: 3px;
    }}

    .flow-connector {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      color: var(--accent-cyan);
      text-align: center;
      padding: 2px 0;
      opacity: 0.85;
      letter-spacing: 0.05em;
    }}

    .lockbox-content-badge {{
      display: inline-block;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.78rem;
      font-weight: 700;
      background: rgba(255, 255, 255, 0.06);
      border: 1px solid rgba(255, 255, 255, 0.12);
      color: #fff;
      padding: 3px 8px;
      border-radius: 4px;
      margin-bottom: 10px;
    }}

    .lockbox-rules-title {{
      font-size: 0.76rem;
      font-weight: 700;
      color: var(--text-dim);
      text-transform: uppercase;
      letter-spacing: 0.06em;
      margin-bottom: 6px;
    }}

    .node-desc {{
      font-size: 0.85rem;
      color: #cbd5e1;
      line-height: 1.45;
    }}

    .text-center {{
      text-align: center;
    }}

    /* ASCII Diagrams & Box Drawing Optimization */
    .ascii-diagram-wrapper {{
      margin: 22px 0;
      border-radius: 10px;
      background: #060910;
      border: 1px solid rgba(0, 240, 255, 0.22);
      box-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
      overflow: hidden;
    }}

    .ascii-diagram-header {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 8px 14px;
      background: #0d121c;
      border-bottom: 1px solid rgba(0, 240, 255, 0.12);
    }}

    .ascii-diagram-tag {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      font-weight: 700;
      color: var(--accent-cyan);
      letter-spacing: 0.08em;
    }}

    .ascii-diagram-scroll-hint {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.7rem;
      color: var(--text-dim);
    }}

    .ascii-diagram-pre {{
      padding: 16px;
      overflow-x: auto;
      font-family: 'JetBrains Mono', 'Courier New', monospace;
      font-size: 0.8rem;
      line-height: 1.15 !important;
      letter-spacing: 0px !important;
      color: #38bdf8;
      background: #05080f;
      white-space: pre !important;
      word-break: normal !important;
      overflow-wrap: normal !important;
      -webkit-overflow-scrolling: touch;
    }}

    /* Tables */
    .table-responsive {{
      overflow-x: auto;
      margin: 24px 0;
      border-radius: 10px;
      border: 1px solid rgba(0, 240, 255, 0.22);
      box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    }}

    table.doc-table {{
      width: 100%;
      border-collapse: collapse;
      background: #0c101a;
      text-align: left;
      font-size: 0.9rem;
    }}

    table.doc-table th {{
      background: #131a29;
      padding: 12px 16px;
      font-weight: 600;
      color: var(--accent-cyan);
      border-bottom: 1px solid rgba(0, 240, 255, 0.25);
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.82rem;
    }}

    table.doc-table td {{
      padding: 12px 16px;
      border-bottom: 1px solid rgba(255, 255, 255, 0.05);
      color: #cbd5e1;
    }}

    table.doc-table tr:hover td {{
      background: rgba(0, 240, 255, 0.04);
    }}

    /* Alerts & Admonitions */
    .doc-alert {{
      margin: 22px 0;
      padding: 16px 20px;
      border-radius: 8px;
      border-left: 4px solid var(--accent-cyan);
      background: rgba(0, 240, 255, 0.06);
    }}

    .doc-alert-caution, .doc-alert-warning {{
      border-left-color: var(--accent-red);
      background: rgba(239, 68, 68, 0.08);
    }}

    .doc-alert-tip {{
      border-left-color: var(--accent-green);
      background: rgba(16, 185, 129, 0.08);
    }}

    .doc-alert .alert-title {{
      font-family: 'JetBrains Mono', monospace;
      font-weight: 700;
      font-size: 0.78rem;
      letter-spacing: 0.08em;
      margin-bottom: 6px;
      color: var(--accent-cyan);
    }}

    .doc-alert-caution .alert-title, .doc-alert-warning .alert-title {{
      color: var(--accent-red);
    }}

    .doc-alert-tip .alert-title {{
      color: var(--accent-green);
    }}

    .doc-quote {{
      border-left: 4px solid rgba(255, 255, 255, 0.2);
      padding: 10px 18px;
      margin: 18px 0;
      color: var(--text-muted);
      font-style: italic;
      background: rgba(255, 255, 255, 0.02);
    }}

    /* Navigation Footer */
    .chapter-nav-footer {{
      display: flex;
      justify-content: space-between;
      gap: 20px;
      margin-top: 60px;
      padding-top: 30px;
      border-top: 1px solid rgba(0, 240, 255, 0.15);
    }}

    .nav-btn {{
      display: flex;
      flex-direction: column;
      gap: 4px;
      padding: 14px 20px;
      border-radius: 10px;
      background: var(--bg-card);
      border: 1px solid var(--border-color);
      color: var(--text-main);
      text-decoration: none;
      min-width: 200px;
      transition: all 0.2s ease;
      cursor: pointer;
    }}

    .nav-btn:hover {{
      border-color: var(--accent-cyan);
      background: var(--bg-card-hover);
      box-shadow: 0 4px 20px rgba(0, 240, 255, 0.2);
      transform: translateY(-2px);
    }}

    .nav-btn.prev {{
      text-align: left;
    }}

    .nav-btn.next {{
      text-align: right;
      margin-left: auto;
    }}

    .nav-btn-sub {{
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.72rem;
      color: var(--text-dim);
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }}

    .nav-btn-title {{
      font-weight: 600;
      font-size: 0.95rem;
      color: var(--accent-cyan);
    }}

    /* Responsive */
    @media (max-width: 900px) {{
      :root {{
        --sidebar-width: 280px;
      }}
      .mobile-menu-btn {{
        display: flex;
      }}
      aside.book-sidebar {{
        position: fixed;
        top: var(--header-height);
        left: 0;
        bottom: 0;
        z-index: 150;
        transform: translateX(-100%);
        transition: transform 0.25s cubic-bezier(0.16, 1, 0.3, 1);
        box-shadow: 10px 0 30px rgba(0, 0, 0, 0.8);
      }}
      aside.book-sidebar.open {{
        transform: translateX(0);
      }}
      .sidebar-footer {{
        display: flex;
      }}
      main.book-content-pane {{
        padding: 24px 20px 60px 20px;
      }}
      .chapter-nav-footer {{
        flex-direction: column;
      }}
      .nav-btn {{
        width: 100%;
        text-align: center !important;
      }}
    }}

    @media (max-width: 768px) {{
      :root {{
        --header-height: 56px;
      }}

      header.book-header {{
        padding: 0 12px;
        gap: 8px;
      }}

      .header-left {{
        gap: 8px;
      }}

      .mobile-menu-btn {{
        width: 36px;
        height: 36px;
        font-size: 1.15rem;
        border-radius: 6px;
      }}

      .brand-group {{
        gap: 8px;
      }}

      .brand-logo-gem {{
        width: 24px;
        height: 24px;
      }}

      .brand-title-wrap {{
        gap: 6px;
      }}

      .brand-title {{
        font-size: 1.05rem;
      }}

      .brand-badge {{
        font-size: 0.65rem;
        padding: 2px 5px;
      }}

      /* Hide inline search bar on mobile */
      .header-search-wrap {{
        display: none;
      }}

      /* Show mobile search button in header */
      .mobile-search-btn {{
        display: flex;
        align-items: center;
        justify-content: center;
        width: 36px;
        height: 36px;
        background: rgba(0, 240, 255, 0.08);
        border: 1px solid var(--border-color);
        color: var(--accent-cyan);
        border-radius: 6px;
        font-size: 0.92rem;
        cursor: pointer;
        transition: all 0.2s ease;
      }}

      .mobile-search-btn:hover {{
        background: rgba(0, 240, 255, 0.18);
        border-color: var(--accent-cyan);
      }}

      .header-links {{
        gap: 8px;
      }}

      /* Hide secondary links on mobile header (they live in sidebar drawer) */
      .nav-link-desktop {{
        display: none;
      }}

      .nav-link-exchange {{
        padding: 6px 10px;
        font-size: 0.78rem;
      }}

      .nav-text-full {{
        display: none;
      }}

      .nav-text-short {{
        display: inline;
      }}

      /* Mobile search drawer dropdown */
      .mobile-search-bar {{
        position: fixed;
        top: var(--header-height);
        left: 0;
        right: 0;
        background: #0d121f;
        border-bottom: 1px solid var(--border-color);
        padding: 10px 14px;
        z-index: 130;
        box-shadow: 0 10px 25px rgba(0, 0, 0, 0.85);
      }}

      .mobile-search-bar.open {{
        display: block;
      }}

      .mobile-search-inner {{
        position: relative;
        display: flex;
        align-items: center;
      }}

      .mobile-search-inner .search-input {{
        padding-right: 36px;
      }}

      .mobile-search-close {{
        position: absolute;
        right: 8px;
        top: 50%;
        transform: translateY(-50%);
        background: none;
        border: none;
        color: var(--text-dim);
        font-size: 1rem;
        cursor: pointer;
        padding: 4px 8px;
      }}

      .mobile-search-close:hover {{
        color: #fff;
      }}

      .mobile-dropdown {{
        position: static;
        margin-top: 8px;
        max-height: 50vh;
      }}

      /* Subheader Breadcrumbs & Meta */
      .chapter-header-meta {{
        display: flex;
        flex-direction: row;
        justify-content: space-between;
        align-items: center;
        gap: 8px;
        margin-bottom: 18px;
        padding-bottom: 10px;
      }}

      .breadcrumbs {{
        font-size: 0.75rem;
        line-height: 1.35;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        flex: 1;
      }}

      .read-time-badge {{
        font-size: 0.72rem;
        padding: 2px 7px;
        white-space: nowrap;
        flex-shrink: 0;
      }}

      main.book-content-pane {{
        padding: 16px 14px 60px 14px;
      }}

      .doc-h1 {{
        font-size: 1.7rem;
        margin-bottom: 14px;
      }}

      .doc-h2 {{
        font-size: 1.25rem;
        margin-top: 28px;
        margin-bottom: 12px;
      }}

      .doc-h3 {{
        font-size: 1.05rem;
      }}

      .doc-para {{
        font-size: 0.95rem;
        line-height: 1.6;
      }}

      /* Diagram Mobile Adaptations */
      .cyber-diagram-box {{
        padding: 14px;
        margin: 18px 0;
      }}
      .diagram-grid-2 {{
        grid-template-columns: 1fr;
        gap: 12px;
      }}
      .outcome-grid {{
        flex-direction: column;
        gap: 12px;
      }}
      .outcome-divider-badge {{
        align-self: center;
      }}
      .ui-mockup-vaults {{
        grid-template-columns: 1fr;
      }}
      .flow-step {{
        padding: 10px 12px;
        gap: 10px;
      }}
      .flow-step-title {{
        font-size: 0.9rem;
      }}
      .flow-step-desc {{
        font-size: 0.8rem;
      }}
      .ascii-diagram-pre {{
        font-size: 0.6rem;
        line-height: 1.12 !important;
        padding: 12px 10px;
      }}
      .ascii-diagram-scroll-hint {{
        color: var(--accent-gold);
      }}
    }}

    @media (max-width: 375px) {{
      header.book-header {{
        padding: 0 8px;
        gap: 6px;
      }}
      .header-left {{
        gap: 6px;
      }}
      .brand-group {{
        gap: 6px;
      }}
      .brand-title {{
        font-size: 0.95rem;
      }}
      .brand-badge {{
        display: none;
      }}
      .mobile-menu-btn, .mobile-search-btn {{
        width: 32px;
        height: 32px;
      }}
    }}
  </style>
</head>
<body>
  <div class="app-layout">
    <!-- Top Header -->
    <header class="book-header">
      <div class="header-left">
        <button class="mobile-menu-btn" onclick="toggleSidebar()" aria-label="Toggle Table of Contents">☰</button>
        <a class="brand-group" onclick="loadChapter('readme')">
          <div class="brand-logo-gem"></div>
          <div class="brand-title-wrap">
            <span class="brand-title">TRUSWAP</span>
            <span class="brand-badge">THE BOOK</span>
          </div>
        </a>
      </div>

      <!-- Search Bar -->
      <div class="header-search-wrap">
        <span class="search-icon">🔍</span>
        <input type="text" class="search-input" id="search-input" placeholder="Search documentation... (Press '/' to focus)" oninput="handleSearch(this.value, 'desktop')">
        <span class="search-shortcut">/</span>
        <div class="search-results-dropdown" id="search-dropdown"></div>
      </div>

      <!-- Quick Links -->
      <div class="header-links">
        <button class="mobile-search-btn" onclick="toggleMobileSearch()" aria-label="Search documentation" title="Search documentation">🔍</button>
        <a href="/" class="header-nav-link nav-link-exchange" title="Open TRUSWAP Atomic Exchange">
          <span>⚡</span>
          <span class="nav-text-full">Exchange</span>
          <span class="nav-text-short">App</span>
        </a>
        <a href="/explorer" class="header-nav-link nav-link-desktop" title="Open Blockchain Explorer">
          <span>🔍</span>
          <span>Explorer</span>
        </a>
        <a href="https://tokenizedrealutility.com" target="_blank" rel="noopener" class="header-nav-link nav-link-desktop" title="Official TRU Network Portal">
          <span>🌐</span>
          <span>TRU Web</span>
        </a>
      </div>
    </header>

    <!-- Mobile Search Bar Dropdown -->
    <div class="mobile-search-bar" id="mobile-search-bar">
      <div class="mobile-search-inner">
        <span class="search-icon">🔍</span>
        <input type="text" class="search-input" id="mobile-search-input" placeholder="Search book chapters..." oninput="handleSearch(this.value, 'mobile')">
        <button class="mobile-search-close" onclick="toggleMobileSearch()" aria-label="Close search">✕</button>
      </div>
      <div class="search-results-dropdown mobile-dropdown" id="mobile-search-dropdown"></div>
    </div>

    <!-- Backdrop for Mobile Sidebar -->
    <div class="sidebar-backdrop" id="sidebar-backdrop" onclick="toggleSidebar()"></div>

    <!-- Main Container -->
    <div class="book-body">
      <!-- Sidebar Navigation -->
      <aside class="book-sidebar" id="book-sidebar">
        <div class="sidebar-section-title">Table of Contents</div>
        <ul class="chapter-list" id="chapter-list"></ul>
        <div class="sidebar-footer">
          <a href="/" class="sidebar-footer-link"><span>⚡</span> TRUSWAP App</a>
          <a href="/explorer" class="sidebar-footer-link"><span>🔍</span> Block Explorer</a>
          <a href="https://tokenizedrealutility.com" target="_blank" rel="noopener" class="sidebar-footer-link"><span>🌐</span> TRU Network Web</a>
        </div>
      </aside>

      <!-- Content Area -->
      <main class="book-content-pane" id="content-pane">
        <div class="content-container">
          <div class="chapter-header-meta">
            <div class="breadcrumbs">
              <span>TRUSWAP</span> &gt; <span>Docs</span> &gt; <span class="current" id="meta-breadcrumb-title">Overview</span>
            </div>
            <div class="read-time-badge" id="meta-read-time">3 min read</div>
          </div>

          <article id="article-content"></article>

          <!-- Navigation Footer -->
          <footer class="chapter-nav-footer">
            <a class="nav-btn prev" id="prev-btn" onclick="navigateChapter(-1)" style="display:none;">
              <span class="nav-btn-sub">← Previous</span>
              <span class="nav-btn-title" id="prev-title"></span>
            </a>
            <a class="nav-btn next" id="next-btn" onclick="navigateChapter(1)" style="display:none;">
              <span class="nav-btn-sub">Next →</span>
              <span class="nav-btn-title" id="next-title"></span>
            </a>
          </footer>
        </div>
      </main>
    </div>
  </div>

  <script>
    const CHAPTERS = {chapters_json};
    let currentChapterIndex = 0;

    function init() {{
      renderSidebar();
      
      // Handle initial route from URL hash
      const hash = window.location.hash.replace('#', '').trim();
      let foundIdx = -1;
      if (hash) {{
        foundIdx = CHAPTERS.findIndex(c => c.id === hash);
      }}
      if (foundIdx === -1) {{
        foundIdx = 0; // default to README
      }}
      
      loadChapterByIndex(foundIdx);

      // Keyboard shortcuts
      window.addEventListener('keydown', (e) => {{
        if (e.target.tagName === 'INPUT') {{
          if (e.key === 'Escape') {{
            e.target.blur();
            closeSearchDropdown();
            const mBar = document.getElementById('mobile-search-bar');
            if (mBar) mBar.classList.remove('open');
          }}
          return;
        }}
        if (e.key === '/') {{
          e.preventDefault();
          if (window.innerWidth <= 768) {{
            toggleMobileSearch();
          }} else {{
            const inp = document.getElementById('search-input');
            if (inp) inp.focus();
          }}
        }} else if (e.key === 'ArrowLeft') {{
          navigateChapter(-1);
        }} else if (e.key === 'ArrowRight') {{
          navigateChapter(1);
        }}
      }});

      // Close search dropdown on click outside
      document.addEventListener('click', (e) => {{
        if (!e.target.closest('.header-search-wrap') && !e.target.closest('.mobile-search-bar') && !e.target.closest('.mobile-search-btn')) {{
          closeSearchDropdown();
          const mBar = document.getElementById('mobile-search-bar');
          if (mBar) mBar.classList.remove('open');
        }}
      }});
    }}

    function renderSidebar() {{
      const list = document.getElementById('chapter-list');
      list.innerHTML = CHAPTERS.map((ch, idx) => `
        <li class="chapter-item">
          <a class="chapter-link ${{idx === currentChapterIndex ? 'active' : ''}}" 
             id="link-${{ch.id}}" 
             onclick="loadChapter('${{ch.id}}')">
            <div class="chapter-link-left">
              <span class="chapter-icon">${{ch.icon}}</span>
              <span class="chapter-title-text">${{ch.title}}</span>
            </div>
            <span class="chapter-num">${{ch.num}}</span>
          </a>
        </li>
      `).join('');
    }}

    function loadChapter(id) {{
      const idx = CHAPTERS.findIndex(c => c.id === id);
      if (idx !== -1) {{
        loadChapterByIndex(idx);
      }}
    }}

    function loadChapterByIndex(idx) {{
      if (idx < 0 || idx >= CHAPTERS.length) return;
      currentChapterIndex = idx;
      const ch = CHAPTERS[idx];

      // Update URL hash without reload
      window.history.replaceState(null, ch.title, '#' + ch.id);

      // Render content
      document.getElementById('meta-breadcrumb-title').textContent = ch.title;
      document.getElementById('meta-read-time').textContent = ch.read_time;
      document.getElementById('article-content').innerHTML = ch.html;

      // Update sidebar active state
      document.querySelectorAll('.chapter-link').forEach(el => el.classList.remove('active'));
      const activeLink = document.getElementById('link-' + ch.id);
      if (activeLink) activeLink.classList.add('active');

      // Scroll reader pane to top
      document.getElementById('content-pane').scrollTop = 0;

      // Update Prev / Next buttons
      const prevBtn = document.getElementById('prev-btn');
      const nextBtn = document.getElementById('next-btn');

      if (idx > 0) {{
        prevBtn.style.display = 'flex';
        document.getElementById('prev-title').textContent = CHAPTERS[idx - 1].title;
      }} else {{
        prevBtn.style.display = 'none';
      }}

      if (idx < CHAPTERS.length - 1) {{
        nextBtn.style.display = 'flex';
        document.getElementById('next-title').textContent = CHAPTERS[idx + 1].title;
      }} else {{
        nextBtn.style.display = 'none';
      }}

      // Close mobile drawer and backdrop if open
      document.getElementById('book-sidebar').classList.remove('open');
      const backdrop = document.getElementById('sidebar-backdrop');
      if (backdrop) backdrop.classList.remove('open');
    }}

    function navigateChapter(delta) {{
      loadChapterByIndex(currentChapterIndex + delta);
    }}

    function toggleSidebar() {{
      const sidebar = document.getElementById('book-sidebar');
      const backdrop = document.getElementById('sidebar-backdrop');
      const isOpen = sidebar.classList.toggle('open');
      if (backdrop) {{
        if (isOpen) {{
          backdrop.classList.add('open');
        }} else {{
          backdrop.classList.remove('open');
        }}
      }}
    }}

    function toggleMobileSearch() {{
      const bar = document.getElementById('mobile-search-bar');
      if (!bar) return;
      const isOpen = bar.classList.toggle('open');
      if (isOpen) {{
        const inp = document.getElementById('mobile-search-input');
        if (inp) {{
          inp.value = '';
          inp.focus();
        }}
      }} else {{
        const dd = document.getElementById('mobile-search-dropdown');
        if (dd) dd.style.display = 'none';
      }}
    }}

    function handleSearch(query, source = 'desktop') {{
      const isMobile = source === 'mobile';
      const dropdown = document.getElementById(isMobile ? 'mobile-search-dropdown' : 'search-dropdown');
      if (!dropdown) return;

      query = query.trim().toLowerCase();
      if (!query || query.length < 2) {{
        dropdown.style.display = 'none';
        dropdown.innerHTML = '';
        return;
      }}

      const matches = [];
      CHAPTERS.forEach((ch, idx) => {{
        const titleMatch = ch.title.toLowerCase().includes(query);
        const textIdx = ch.raw_text.toLowerCase().indexOf(query);
        if (titleMatch || textIdx !== -1) {{
          let snippet = "";
          if (textIdx !== -1) {{
            const start = Math.max(0, textIdx - 40);
            const end = Math.min(ch.raw_text.length, textIdx + 80);
            snippet = (start > 0 ? "..." : "") + ch.raw_text.substring(start, end).replace(/\\s+/g, ' ') + "...";
          }} else {{
            snippet = ch.title;
          }}
          matches.push({{ chapter: ch, idx: idx, snippet: snippet }});
        }}
      }});

      if (matches.length === 0) {{
        dropdown.innerHTML = '<div style="padding: 12px 14px; font-size: 0.85rem; color: var(--text-dim);">No matches found for "' + htmlEscape(query) + '"</div>';
      }} else {{
        dropdown.innerHTML = matches.map(m => `
          <div class="search-result-item" onclick="selectSearchResult('${{m.chapter.id}}')">
            <div class="search-result-title">${{m.chapter.icon}} ${{m.chapter.title}}</div>
            <div class="search-result-snippet">${{htmlEscape(m.snippet)}}</div>
          </div>
        `).join('');
      }}
      dropdown.style.display = 'block';
    }}

    function selectSearchResult(id) {{
      closeSearchDropdown();
      const dInput = document.getElementById('search-input');
      if (dInput) dInput.value = '';
      const mInput = document.getElementById('mobile-search-input');
      if (mInput) mInput.value = '';
      const mBar = document.getElementById('mobile-search-bar');
      if (mBar) mBar.classList.remove('open');
      loadChapter(id);
    }}

    function closeSearchDropdown() {{
      const dDropdown = document.getElementById('search-dropdown');
      if (dDropdown) dDropdown.style.display = 'none';
      const mDropdown = document.getElementById('mobile-search-dropdown');
      if (mDropdown) mDropdown.style.display = 'none';
    }}

    function htmlEscape(str) {{
      return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    }}

    function copyCode(btn) {{
      const code = btn.closest('.code-block-wrapper').querySelector('code').innerText;
      navigator.clipboard.writeText(code).then(() => {{
        const orig = btn.innerText;
        btn.innerText = "✓ Copied!";
        btn.style.color = "var(--accent-green)";
        btn.style.borderColor = "var(--accent-green)";
        setTimeout(() => {{
          btn.innerText = orig;
          btn.style.color = "";
          btn.style.borderColor = "";
        }}, 2000);
      }}).catch(err => {{
        console.error("Clipboard copy failed", err);
      }});
    }}

    function isExternalUrl(url) {{
      if (!url || typeof url !== 'string') return false;
      if (!url.startsWith('http://') && !url.startsWith('https://')) return false;
      try {{
        const parsed = new URL(url, window.location.href);
        const host = parsed.hostname;
        const isLocal = host === '127.0.0.1' || host === 'localhost' || host === window.location.hostname;
        return !isLocal;
      }} catch (_) {{
        return false;
      }}
    }}

    function openExternalUrl(url) {{
      if (!url) return;
      try {{
        fetch('/api/open_browser', {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{ url: url }})
        }}).catch(e => {{
          console.warn("[ExternalUrl] open_browser error:", e);
        }});
      }} catch (_) {{}}

      const isAndroid = navigator.userAgent.includes('TRUSWAP_APK') || /Android/i.test(navigator.userAgent);
      if (!isAndroid) {{
        window.open(url, '_blank', 'noopener,noreferrer');
      }}
    }}
    window.openExternalUrl = openExternalUrl;

    document.addEventListener('click', function(e) {{
      const link = e.target.closest('a');
      if (!link) return;

      const rawHref = link.getAttribute('href');
      if (!rawHref || rawHref.startsWith('#') || rawHref.startsWith('javascript:')) {{
        return;
      }}

      const href = link.href;
      if (isExternalUrl(href)) {{
        e.preventDefault();
        e.stopPropagation();
        openExternalUrl(href);
      }}
    }}, true);

    window.onload = init;
  </script>
</body>
</html>
'''

# Write to docs/index.html
index_path = os.path.join(docs_dir, "index.html")
with open(index_path, "w", encoding="utf-8") as fp:
    fp.write(html_template)

# Also write to src/ui/docs.html for the embedded server
ui_docs_path = "/home/user/git_test/projects/tru/repos/truswap/src/ui/docs.html"
with open(ui_docs_path, "w", encoding="utf-8") as fp:
    fp.write(html_template)

print(f"Generated {index_path} ({len(html_template)} bytes)")
print(f"Generated {ui_docs_path} ({len(html_template)} bytes)")
