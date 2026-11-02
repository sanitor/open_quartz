#!/usr/bin/env node
import { readFile, writeFile, readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Stable IDs match checkpoint names; descriptions explain the observable contract.
export const UI_CASES = [
  ['default-empty-start-stopped', '默认空白启动', '画布没有节点和连线，播放停止，预览无图像。', ['01-']],
  ['open-empty-project-stopped', '打开空项目', '用项目路径重新启动，确认打开正确文件，画布为空且播放停止。', []],
  ['menu-add-default-editable-shader-stopped', '菜单添加 Shader', '通过菜单创建可编辑的自定义 Shader，源码存在，保持停止。', ['02-']],
  ['single-node-drag-positive-x', '单节点向右拖动', '松手后的屏幕落点和世界坐标正确，节点不回中。', ['03-drag-positive-x']],
  ['single-node-drag-negative-x', '单节点向左拖动', '反向拖动保持落点，播放仍停止。', ['03-drag-negative-x']],
  ['single-node-drag-positive-y', '单节点向下拖动', '垂直拖动保持落点，播放仍停止。', ['03-drag-positive-y']],
  ['single-node-drag-negative-y', '单节点向上拖动', '反向垂直拖动保持落点，播放仍停止。', ['03-drag-negative-y']],
  ['zoom-out-then-drag', '缩小后拖动', '缩小确实改变节点尺寸，拖动按缩放比例换算世界坐标并保持落点。', ['04-']],
  ['zoom-in-then-drag', '放大后拖动', '放大确实改变节点尺寸，拖动按缩放比例换算世界坐标并保持落点。', ['05-']],
  ['fit-then-drag', 'Fit 后拖动', '点击 Fit 后再拖动，节点保持在鼠标落点。', ['06-']],
  ['undo-redo-node-drag', '撤销与重做拖动', 'Undo 恢复拖动前的位置，Redo 恢复拖动后的位置。', ['07-', '08-']],
  ['edit-shader-label-and-source', '编辑并提交 Shader', '修改节点名称与蓝色输出源码，点击 Apply，确认草稿清除且 SDK 提交成功。', ['09-']],
  ['menu-add-renderer-connect-stopped', '添加 Renderer 并连接', '添加并拖开 Renderer，点击两端口建立唯一连线，保持停止。', ['10-']],
  ['play-real-renderer-preview', '播放与真实 GPU 输出', '点击 Play，确认两个节点缩略图就绪，预览采样至少 256 像素且蓝色比例不低于 80%。', ['11-']],
  ['stop-playback', '停止播放', '点击 Stop，确认播放停止，节点和连接保留。', ['12-']],
  ['save-project-through-file-menu', '通过文件菜单保存', '点击 Save，确认落盘包含两个节点、一条连线、修改后的名称和源码。', []],
  ['restart-same-project-retains-graph-parameters-stopped', '重启并恢复项目', '重开同一文件，核对节点 ID、名称、位置、连线和源码，确认保持停止。', ['13-', '14-']],
].map(([id, title, description, screenshotPrefixes]) => ({ id, title, description, screenshotPrefixes }));

export const STATUS_LABELS = { passed: '通过', failed: '失败', incomplete: '未完成', not_run: '未执行' };
const escapeHtml = (value = '') => String(value).replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[char]);
const duration = (start, finish) => {
  const ms = Date.parse(finish) - Date.parse(start);
  return Number.isFinite(ms) && ms >= 0 ? `${(ms / 1000).toFixed(2)} 秒` : '—';
};
const fileName = (path) => String(path).replaceAll('\\', '/').split('/').pop();
const assetUrl = (path) => encodeURIComponent(fileName(path));

export function reportModel(run = {}, scenarios = {}) {
  const source = run.scenarios || scenarios;
  const steps = source.steps || [];
  const screenshots = source.screenshots || [];
  const catalog = [...(source.caseDefinitions || UI_CASES)];
  for (const step of steps) {
    if (!catalog.some((entry) => entry.id === step.name)) catalog.push({ id: step.name, title: step.title || step.name, description: step.description || '自定义检查点', screenshotPrefixes: [] });
  }
  const cases = catalog.map((entry) => {
    const step = steps.find((item) => item.name === entry.id);
    const status = !step ? 'not_run' : step.passed === true ? 'passed' : step.passed === false ? 'failed' : 'incomplete';
    return {
      ...entry, ...step, status,
      screenshots: step?.screenshots || screenshots.filter((path) => (entry.screenshotPrefixes || []).some((prefix) => fileName(path).startsWith(prefix))),
      duration: duration(step?.startedAt, step?.finishedAt),
    };
  });
  const counts = Object.fromEntries(Object.keys(STATUS_LABELS).map((status) => [status, cases.filter((entry) => entry.status === status).length]));
  const passed = run.passed === true && counts.passed === cases.length;
  return { run, scenarios: source, cases, counts, passed, error: run.error || source.error, duration: duration(run.startedAt || source.startedAt, run.finishedAt || source.finishedAt) };
}

function evidence(entry) {
  const detail = entry.details;
  if (!detail) return '';
  if (detail.pixels?.verified && detail.pixels.bluePixels !== undefined) return `GPU 采样 ${detail.pixels.bluePixels.toLocaleString()} / ${detail.pixels.totalPixels.toLocaleString()} 个蓝色像素（${(detail.pixels.blueRatio * 100).toFixed(1)}%）`;
  if (detail.pixels?.verified || detail.pixels?.metrics?.verified) {
    const pixels = detail.pixels.metrics || detail.pixels;
    const oracle = typeof pixels.oracle === 'object' ? pixels.oracle.kind : pixels.oracle;
    return `像素验证：${oracle || '独立参考'}；匹配 ${pixels.matchedPixels ?? '—'} / ${pixels.totalPixels ?? '—'}；平均误差 ${pixels.meanError?.toFixed(2) ?? '—'}`;
  }
  if (detail.before && detail.after && detail.target) return `世界坐标 (${detail.before.x}, ${detail.before.y}) → (${detail.after.x}, ${detail.after.y})；屏幕落点 (${detail.after.center?.x?.toFixed(1)}, ${detail.after.center?.y?.toFixed(1)})`;
  if (detail.undone && detail.redone) return `Undo (${detail.undone.x}, ${detail.undone.y})；Redo (${detail.redone.x}, ${detail.redone.y})`;
  const state = detail.status || detail;
  if (state.Playback) return `播放：${state.Playback}；节点：${state.Nodes}；连线：${state.Connections}`;
  if (detail.label && detail.source) return `已提交名称“${detail.label}”与蓝色 Shader 源码`;
  if (detail.nodes && detail.edges) return `核对 ${detail.nodes.length} 个节点、${detail.edges.length} 条连线及项目参数`;
  return '';
}

export function renderHtml(model, artifacts = []) {
  const { run, scenarios, cases, counts, passed, error } = model;
  const links = (paths) => paths.map((path) => `<a href="${assetUrl(path)}">${escapeHtml(fileName(path))}</a>`).join(' · ');
  const failureImage = artifacts.find((name) => name === 'failure.png');
  const overview = cases.map((entry, index) => `<tr data-case="case-${index}"><td>${index + 1}</td><th scope="row"><a href="#case-${index}">${escapeHtml(entry.title)}</a></th><td class="${entry.status}"><span class="badge">${STATUS_LABELS[entry.status]}</span></td><td>${entry.duration}</td></tr>`).join('');
  const rows = cases.map((entry, index) => `<article id="case-${index}" class="case ${entry.status}" data-status="${entry.status}" data-search="${escapeHtml(`${entry.title} ${entry.description} ${entry.id}`)}">
    <div class="case-heading"><h2>${index + 1}. ${escapeHtml(entry.title)}</h2><span class="badge">${STATUS_LABELS[entry.status]}</span><time>${entry.duration}</time></div>
    <p>${escapeHtml(entry.description)}</p><small>${escapeHtml(entry.id)}</small>
    ${entry.status === 'not_run' ? '<p class="muted">前序检查失败或应用未能启动，尚未执行此项。</p>' : ''}
    ${entry.status === 'incomplete' ? '<p class="muted">已经开始，但未获得完成结果；请查看运行错误和日志。</p>' : ''}
    ${evidence(entry) ? `<p class="evidence">${escapeHtml(evidence(entry))}</p>` : ''}
    ${entry.details?.uniforms ? `<p>参数：<code>${escapeHtml(JSON.stringify(entry.details.uniforms))}</code></p>` : ''}
    ${entry.details?.phases?.length ? `<table><thead><tr><th>验证阶段</th><th>结果</th><th>耗时</th></tr></thead><tbody>${entry.details.phases.map((phase) => `<tr><th>${escapeHtml(phase.title || phase.id)}</th><td>${phase.passed === true ? '通过' : phase.passed === false ? '失败' : '未完成'}</td><td>${duration(phase.startedAt, phase.finishedAt)}</td></tr>`).join('')}</tbody></table>` : ''}
    ${entry.error ? `<pre class="error">${escapeHtml(entry.error)}</pre>` : ''}
    ${entry.screenshots.length ? `<details><summary>查看 ${entry.screenshots.length} 张截图</summary><div class="screenshots">${entry.screenshots.map((path) => `<figure><a href="${assetUrl(path)}"><img loading="lazy" src="${assetUrl(path)}" alt="${escapeHtml(entry.title)}"></a><figcaption>${escapeHtml(fileName(path))}</figcaption></figure>`).join('')}</div></details>` : ''}
    ${entry.details ? `<details><summary>技术详情</summary><pre>${escapeHtml(JSON.stringify(entry.details, null, 2))}</pre></details>` : ''}
  </article>`).join('\n');
  return `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>OpenQuartz UI 测试报告 · ${passed ? '通过' : '失败'}</title>
<style>
:root{font-family:system-ui,-apple-system,sans-serif;color:#172033;background:#f3f5f8;line-height:1.6;color-scheme:light}*{box-sizing:border-box}body{margin:0}main{max-width:1060px;margin:36px auto;padding:0 24px}h1{margin:0;font-size:28px}h2{margin:0;font-size:17px}header,.case,.failure{background:white;border:1px solid #dce1e8;border-radius:10px;padding:22px;margin-bottom:16px}header{border-top:5px solid ${passed ? '#167145' : '#b42318'}}p{margin:8px 0}a{color:#0759a5;overflow-wrap:anywhere}.muted,small,time{color:#586579}small{overflow-wrap:anywhere}.metrics{display:flex;gap:12px;flex-wrap:wrap;margin-top:18px}.metric{background:#f3f5f8;padding:10px 18px;border-radius:7px}.metric strong{font-size:22px;margin-right:6px}.toolbar{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin:20px 0}input,select{padding:9px;border:1px solid #b7c1cf;border-radius:5px;font:inherit}input{flex:1;min-width:160px}.case-heading{display:flex;align-items:center;gap:14px;flex-wrap:wrap}.case-heading h2{flex:1;min-width:180px}.badge{border-radius:20px;padding:2px 12px;font-size:13px;font-weight:650}.passed .badge{background:#e6f4eb;color:#16643d}.failed .badge{background:#ffebe9;color:#a61e17}.incomplete .badge{background:#fff2cc;color:#755200}.not_run .badge{background:#eceff3;color:#505c70}.case.failed{border-left:5px solid #b42318}.evidence{color:#16643d}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f3f5f8;padding:14px;font-size:12px;border-radius:5px;max-height:440px;overflow:auto}.error{color:#9c211a;background:#fff1ef}summary{cursor:pointer;color:#0759a5;padding:8px 0}.screenshots{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:12px}figure{margin:0}img{max-width:100%;border:1px solid #dce1e8;border-radius:4px}figcaption{font-size:12px;color:#586579;overflow-wrap:anywhere}footer{font-size:13px;color:#586579;margin:28px 0}[hidden]{display:none!important}@media print{.toolbar{display:none}details{break-inside:avoid}.case{break-inside:avoid}}
table{width:100%;border-collapse:collapse;background:white;margin:0 0 24px;border:1px solid #dce1e8}caption{text-align:left;font-weight:650;padding:0 0 8px}td,th{text-align:left;padding:6px 14px;border-bottom:1px solid #e8ecf1;font-size:14px}thead{background:#e9edf3}tbody th{font-weight:500}td:first-child{width:50px;color:#586579}.case{scroll-margin-top:18px}
</style></head><body><main>
<header><h1>UI 自动化测试：${passed ? '全部通过' : '未全部通过'}</h1><p>OpenQuartz · ${cases.length} 个 case · 耗时 ${model.duration}</p><p class="muted">开始：${escapeHtml(run.startedAt || scenarios.startedAt || '未知')}<br>结束：${escapeHtml(run.finishedAt || scenarios.finishedAt || '未知')}</p>
<div class="metrics">${Object.entries(STATUS_LABELS).map(([status, label]) => `<div class="metric"><strong>${counts[status]}</strong>${label}</div>`).join('')}</div></header>
${error ? `<section class="failure"><h2>运行失败原因</h2><pre class="error">${escapeHtml(String(error).split('\n')[0])}</pre><details><summary>完整错误与失败截图</summary><pre class="error">${escapeHtml(error)}</pre>${failureImage ? `<a href="${assetUrl(failureImage)}"><img src="${assetUrl(failureImage)}" alt="失败时的窗口截图"></a>` : ''}</details><p>${links(artifacts.filter((name) => /^(failure|application-)/.test(name)))}</p></section>` : ''}
<div class="toolbar"><label for="status">显示</label><select id="status"><option value="all">全部 case</option>${Object.entries(STATUS_LABELS).map(([status, label]) => `<option value="${status}">${label} (${counts[status]})</option>`).join('')}</select><input id="search" type="search" aria-label="搜索 case" placeholder="搜索名称、验证内容或 case ID"><span id="visible-count">${cases.length} 项</span></div>
<table><caption>Case 结果总览 · 点击名称查看断言和截图</caption><thead><tr><th scope="col">#</th><th scope="col">Case</th><th scope="col">结果</th><th scope="col">耗时</th></tr></thead><tbody>${overview}</tbody></table>
${rows}
<footer><p>附件：${links(artifacts.filter((name) => /\.(json|log)$/.test(name)))}</p><p>菜单、按钮、端口和拖动通过 Slint 指针事件验证；节点选择与文本编辑使用辅助功能接口。GPU 像素检查读取真实渲染窗口快照。原生系统文件对话框、IME 和摄像头权限不在此套件范围内。</p><p>本页可离线查看，截图和附件与 report.html 保持在同一目录。</p></footer>
</main><script>
const filter=document.getElementById('status'),search=document.getElementById('search');
function update(){let count=0;for(const row of document.querySelectorAll('.case')){row.hidden=!(filter.value==='all'||row.dataset.status===filter.value)||!row.dataset.search.toLowerCase().includes(search.value.toLowerCase());document.querySelector('[data-case="'+row.id+'"]').hidden=row.hidden;if(!row.hidden)count++;}document.getElementById('visible-count').textContent=count+' 项';}
filter.addEventListener('change',update);search.addEventListener('input',update);
</script></body></html>`;
}

export function consoleSummary(model) {
  return [`[slint-ui] ${model.cases.length} 个 case：${Object.entries(STATUS_LABELS).map(([status, label]) => `${model.counts[status]} ${label}`).join(' / ')}`,
    ...model.cases.map((entry, index) => `  ${String(index + 1).padStart(2)} [${STATUS_LABELS[entry.status]}] ${entry.title} (${entry.duration})`)].join('\n');
}

export async function writeHtmlReport(directory, run, scenarios) {
  const outputDir = resolve(directory);
  const model = reportModel(run, scenarios);
  const owned = new Set(['run.json', ...(model.scenarios.startedAt ? ['result.json'] : []),
    ...(model.scenarios.screenshots || []).map(fileName), ...(run?.launches || []).map((launch) => fileName(launch.logPath))]);
  for (const path of model.scenarios.artifacts || []) owned.add(fileName(path));
  for (const entry of model.cases) for (const path of entry.artifacts || []) owned.add(fileName(path));
  if (owned.has('failure.png')) owned.add('failure-tree.json');
  if (model.cases.some((entry) => entry.id === 'save-project-through-file-menu' && entry.status === 'passed')) {
    for (const name of ['saved-tree.json', 'saved-project-observed.json', fileName(model.scenarios.projectPath || '')]) owned.add(name);
  }
  if (model.cases.some((entry) => entry.id === 'restart-same-project-retains-graph-parameters-stopped' && entry.status === 'passed')) owned.add('reopened-tree.json');
  const artifacts = (await readdir(outputDir)).filter((name) => owned.has(name)).sort();
  const path = join(outputDir, 'report.html');
  await writeFile(path, renderHtml(model, artifacts));
  return { path, model };
}

async function readJson(path) {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return undefined; throw error; }
}

export async function regenerateReport(directory) {
  const outputDir = resolve(directory);
  const run = await readJson(join(outputDir, 'run.json'));
  const scenarios = await readJson(join(outputDir, 'result.json'));
  if (!run && !scenarios) throw new Error(`No run.json or result.json found in ${outputDir}`);
  // Prefer a newer standalone result if a hard termination left an older run.json.
  if (scenarios && (!run || Date.parse(scenarios.startedAt) > Date.parse(run.finishedAt || run.startedAt))) {
    return writeHtmlReport(outputDir, { passed: scenarios.passed, startedAt: scenarios.startedAt,
      finishedAt: scenarios.finishedAt, error: scenarios.error, scenarios });
  }
  const current = run && scenarios && Date.parse(scenarios.startedAt) < Date.parse(run.startedAt) ? undefined : scenarios;
  return writeHtmlReport(outputDir, run, current);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory] = process.argv.slice(2);
  if (!directory) throw new Error('Usage: node scripts/slint-ui-report.mjs <run-directory>');
  const { path, model } = await regenerateReport(directory);
  console.log(`${consoleSummary(model)}\n[slint-ui] HTML report: ${path}`);
}
