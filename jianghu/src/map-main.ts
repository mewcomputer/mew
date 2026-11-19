import { loadAgentArtwork } from './agent-sprites.ts';
import { createAgentTraffic } from './agent-traffic.ts';
import { createDistrict } from './district.ts';
import { createAgentStore, demoAgents, statusLabels, type AgentStatus } from './agents.ts';
import { mountMapScene } from './map-scene.ts';
import { isCameraControl } from './scene-view.ts';
import { requiredElement } from './dom.ts';

const root = requiredElement(document, '#jianghu', HTMLElement);
const find = (selector: string) => requiredElement(root, selector, HTMLElement);
const lifetime = new AbortController();
const { signal } = lifetime;
const onClick = (node: HTMLElement, handler: () => void) => node.addEventListener('click', handler, { signal });
const store = createAgentStore(demoAgents);
const district = createDistrict();
const traffic = createAgentTraffic(district, store.all());
const list = find('.agent-list');
const unitLayer = find('.map-units');
const detail = find('.agent-detail');
const approvalPanel = find('.approval-panel');
const nextButton = requiredElement(root, '[data-next]', HTMLButtonElement);
const filter = requiredElement(root, '[data-filter]', HTMLButtonElement);
const demo = requiredElement(root, '[data-demo]', HTMLButtonElement);
const closeButton = requiredElement(root, '[data-close]', HTMLButtonElement);
const canvas = requiredElement(root, 'canvas', HTMLCanvasElement);
const cameraControls = find('.camera-controls');
const notice = find('.world-note');
const announcement = find('.world-announcement');
const units = new Map<string, HTMLButtonElement>();
const rows = new Map<string, HTMLButtonElement>();
const glyphs = { idle: '•', running: '◎', waiting: '⌛', needs_approval: '印', done: '✓', failed: '!' };
let scene: ReturnType<typeof mountMapScene> | undefined;
let activeId: string | undefined;
let returnFocus: HTMLButtonElement | undefined;
let lastRouted: string | undefined;
let demoRun = 0;
let motionFrame = 0,
  previousStep = 0,
  gateClosed = false;

for (const agent of store.all()) {
  const unit = document.createElement('button');
  unit.type = 'button';
  unit.className = 'map-unit';
  unit.hidden = true;
  const badge = document.createElement('span');
  badge.className = 'map-status';
  badge.setAttribute('aria-hidden', 'true');
  unit.append(badge);
  onClick(unit, () => openAgent(agent.id, unit));
  unitLayer.append(unit);
  units.set(agent.id, unit);

  const row = document.createElement('li');
  const button = document.createElement('button');
  button.type = 'button';
  const name = document.createElement('strong');
  name.textContent = agent.name;
  button.append(name, document.createElement('span'));
  onClick(button, () => openAgent(agent.id, button));
  row.append(button);
  list.append(row);
  rows.set(agent.id, button);
}

function paint() {
  const agents = store.all();
  const pending = store.pending();
  const counts = [
    [pending.length, pending.length === 1 ? 'needs you' : 'need you'],
    [agents.filter((agent) => agent.status === 'running').length, 'running'],
    [agents.filter((agent) => agent.status === 'waiting').length, 'waiting'],
  ];
  find('.agent-counts').textContent = counts.map(([count, label]) => count + ' ' + label).join(' · ');
  nextButton.disabled = pending.length === 0;
  for (const agent of agents) {
    const unit = units.get(agent.id);
    const row = rows.get(agent.id);
    if (!unit || !row) continue;
    const status = statusLabels[agent.status];
    unit.dataset.state = row.dataset.state = agent.status;
    unit.toggleAttribute('data-oldest', pending[0]?.id === agent.id);
    unit.toggleAttribute('data-muted', filter.getAttribute('aria-pressed') === 'true' && !agent.approval);
    if (unit.firstElementChild) unit.firstElementChild.textContent = glyphs[agent.status];
    unit.setAttribute('aria-label', agent.name + ', ' + status);
    unit.title = [agent.name + ' · ' + agent.role, agent.task, agent.activity].join('\n');
    if (row.lastElementChild) row.lastElementChild.textContent = status;
    row.toggleAttribute('data-selected', activeId === agent.id);
  }
  scene?.update();
  detail.hidden = !activeId;
  if (!activeId) return;
  const agent = store.get(activeId);
  if (!agent) return;
  find('#detail-title').textContent = agent.name + ' · ' + agent.role;
  find('.agent-task').textContent = agent.task;
  find('.agent-activity').textContent = agent.activity;
  approvalPanel.hidden = !agent.approval;
  if (agent.approval) find('.approval-summary').textContent = agent.approval.summary;
}

function openAgent(id: string, trigger: HTMLButtonElement) {
  const agent = store.get(id);
  if (!agent) return;
  if (!activeId) returnFocus = trigger;
  activeId = id;
  if (agent.approval) lastRouted = id;
  paint();
  scene?.focus(id);
  closeButton.focus();
}
function closeAgent() {
  if (!activeId) return;
  const row = rows.get(activeId);
  activeId = undefined;
  paint();
  scene?.back();
  const trigger = returnFocus && !returnFocus.hidden && !returnFocus.disabled ? returnFocus : row;
  trigger?.focus();
  returnFocus = undefined;
}
function nextApproval() {
  const pending = store.pending();
  if (!pending.length) return;
  const index = pending.findIndex((agent) => agent.id === lastRouted);
  openAgent(pending[(index + 1) % pending.length].id, nextButton);
}

function animateTravel(now: number) {
  motionFrame = 0;
  const seconds = previousStep ? Math.min(0.05, (now - previousStep) / 1000) : 0;
  previousStep = now;
  traffic.step(seconds);
  scene?.update();
  if (traffic.moving) motionFrame = requestAnimationFrame(animateTravel);
  else previousStep = 0;
}
function startTravel() {
  if (!motionFrame && traffic.moving) motionFrame = requestAnimationFrame(animateTravel);
}
function updateActivity(id: string, activity: string, status: AgentStatus = 'running') {
  return store.dispatch({ type: 'agent.status', id, status, activity });
}
function moveToSluice() {
  const closeSluice = () => {
    gateClosed = true;
    scene?.setGate(true);
    updateActivity('mei', 'Closed the sluice. The upstream level is rising.', 'done');
    announcement.textContent = 'Mei closed the sluice.';
  };
  const approaches = [
    [-2.25, -1.25],
    [-1.75, -1.25],
    [-2.25, -0.75],
  ];
  if (approaches.some(([x, z]) => traffic.walk('mei', x, z, closeSluice))) {
    if (!gateClosed) updateActivity('mei', 'Walking to the sluice controls.');
    startTravel();
  } else {
    updateActivity('mei', 'The path to the sluice is blocked. The gate is still open.', 'waiting');
    announcement.textContent = 'Mei could not reach the sluice.';
  }
}

const unsubscribe = store.subscribe((event) => {
  paint();
  if (event.type === 'approval.resolved' && event.id === 'lin') demo.disabled = false;
  if (event.type === 'approval.requested') {
    const agent = store.get(event.id);
    if (agent) announcement.textContent = agent.name + ' needs approval.';
  }
});
onClick(nextButton, nextApproval);
onClick(closeButton, closeAgent);
onClick(filter, () => {
  filter.setAttribute('aria-pressed', String(filter.getAttribute('aria-pressed') !== 'true'));
  paint();
});
for (const button of root.querySelectorAll<HTMLButtonElement>('button[data-outcome]'))
  onClick(button, () => {
    const agent = activeId ? store.get(activeId) : undefined;
    const outcome = button.dataset.outcome;
    if (!agent?.approval || (outcome !== 'approved' && outcome !== 'declined')) return;
    store.dispatch({
      type: 'approval.resolved',
      id: agent.id,
      approvalId: agent.approval.id,
      outcome,
    });
    const updated = store.get(agent.id);
    if (updated) announcement.textContent = updated.activity;
    if (agent.approval.id === 'sluice-review' && outcome === 'approved') moveToSluice();
    closeButton.focus();
  });
onClick(demo, () => {
  if (traffic.get('lin')?.path.length || traffic.get('shen')?.path.length || store.get('lin')?.approval)
    return;
  const id = ++demoRun;
  let remaining = 2;
  demo.disabled = true;
  updateActivity('lin', 'Carrying the canal survey to the bridge.');
  updateActivity('shen', 'Walking to the bridge to collect the survey.');
  announcement.textContent = 'Lin and Shen are heading to the bridge.';
  const arrived = () => {
    if (--remaining || !traffic.near('lin', 'shen')) return;
    scene?.handoff('lin', 'shen');
    updateActivity('shen', 'Received the canal survey from Lin.', 'done');
    updateActivity('lin', 'Handed the canal survey to Shen.');
    store.dispatch({
      type: 'approval.requested',
      id: 'lin',
      approvalId: 'survey-' + id,
      at: Date.now(),
      summary: 'Share the completed canal survey with the demo workspace.',
    });
    announcement.textContent = 'Lin handed the survey to Shen and requested approval to share it.';
  };
  const lin = traffic.walk('lin', -0.75, 1.25, arrived);
  const shen = traffic.walk('shen', 0.25, 1.25, arrived);
  if (!lin || !shen) {
    demo.disabled = false;
    announcement.textContent = 'A route to the bridge is blocked.';
  }
  startTravel();
});

for (const button of cameraControls.querySelectorAll<HTMLButtonElement>('button[data-camera]')) {
  onClick(button, () => {
    const name = button.dataset.camera;
    if (isCameraControl(name)) scene?.control(name);
  });
}
document.addEventListener('keydown', (event) => {
  if (
    event.defaultPrevented ||
    event.ctrlKey ||
    event.metaKey ||
    event.altKey ||
    (event.target instanceof Element && event.target.matches('input, textarea, select, [contenteditable]'))
  )
    return;
  switch (event.key.toLowerCase()) {
    case 'n':
      if (!nextButton.disabled) {
        event.preventDefault();
        nextApproval();
      }
      break;
    case 'escape':
      closeAgent();
      break;
    case 'r':
      scene?.control('reset');
      break;
    case 'q':
      scene?.control('left');
      break;
    case 'e':
      scene?.control('right');
      break;
    case '+':
    case '=':
      scene?.control('in');
      break;
    case '-':
      scene?.control('out');
      break;
  }
}, { signal });

function showFallback(fallback: boolean) {
  notice.hidden = !fallback;
  notice.textContent = 'Use the agent list to open a thread or approval.';
  canvas.hidden = fallback;
  if (fallback) for (const unit of units.values()) unit.hidden = true;
  cameraControls.hidden = fallback;
  find('.fallback').hidden = !fallback;
}
window.addEventListener('pagehide', (event) => {
  if (event.persisted) return;
  lifetime.abort();
  cancelAnimationFrame(motionFrame);
  unsubscribe();
  scene?.dispose();
}, { signal });
paint();
Promise.all([
  import('three'),
  loadAgentArtwork(root.dataset.spriteSheet),
])
  .then(([T, artwork]) => {
    if (signal.aborted) return;
    scene = mountMapScene(T, {
      canvas,
      cast: store.all(),
      appearance(id) {
        const unit = units.get(id);
        if (!unit) return;
        return { working: unit.dataset.state === 'running', muted: unit.hasAttribute('data-muted') };
      },
      onProject(id, x, y, visible) {
        const unit = units.get(id);
        if (!unit) return;
        unit.hidden = !visible;
        unit.style.left = x + 'px';
        unit.style.top = y + 'px';
      },
      artwork,
      district,
      traffic,
      unavailable: showFallback,
    });
    if (gateClosed) scene.setGate(true);
    showFallback(false);
    if (activeId) scene.focus(activeId);
  })
  .catch((error) => {
    showFallback(true);
    console.warn('Jianghu map initialization', error);
  });
