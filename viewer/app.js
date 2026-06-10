const SELECT_MODE = "select";
const PAN_MODE = "pan";
const ZERO_COORD_MODE = "zero";
const RAW_COORD_MODE = "raw";
const PLOT_PADDING = { left: 74, right: 22, top: 34, bottom: 76 };
const CLICK_DISTANCE_PX = 5;
const MAX_CONTIGUOUS_DX = 2.0;
const MAX_CONTIGUOUS_LOG_GAP_SECONDS = 1.5;

const state = {
  payload: null,
  selectedRunId: null,
  selectedRunKey: null,
  selectedPointIndex: null,
  interactionMode: SELECT_MODE,
  coordinateMode: ZERO_COORD_MODE,
  truncateEndAlignment: false,
  interval: {
    startX: null,
    endX: null,
  },
  previewInterval: null,
  view: null,
  drag: null,
  pointCache: [],
  searchTask: {
    id: null,
    running: false,
    timer: null,
  },
  runListScrollTop: 0,
  canvas: {
    width: 0,
    height: 0,
    dpr: window.devicePixelRatio || 1,
  },
};

const editorState = {
  tab: "param",
  cells: [],
  prefixLength: 0,
  selectionStart: null,
  selectionEnd: null,
  dragSelecting: false,
  loadedRunId: null,
  manualEditor: false,
  launchSlimeBlockX: -1,
  loadedLaunchMode: "water",
  startEntityIdMod4: 0,
  startInitialTickCount: 0,
  preservedStart: null,
  preservedLaunchConfig: null,
  preservedLaunch: null,
};

window.__viewerState = state;

const chartCanvas = document.getElementById("chartCanvas");
const chartContext = chartCanvas.getContext("2d");
const refreshButton = document.getElementById("refreshButton");
const exportCsvButton = document.getElementById("exportCsvButton");
const resetZoomButton = document.getElementById("resetZoomButton");
const selectModeButton = document.getElementById("selectModeButton");
const panModeButton = document.getElementById("panModeButton");
const zeroCoordButton = document.getElementById("zeroCoordButton");
const rawCoordButton = document.getElementById("rawCoordButton");
const preciseEndButton = document.getElementById("preciseEndButton");
const truncatedEndButton = document.getElementById("truncatedEndButton");
const toolbarNote = document.getElementById("toolbarNote");
const applyIntervalButton = document.getElementById("applyIntervalButton");
const resetIntervalButton = document.getElementById("resetIntervalButton");
const intervalStartInput = document.getElementById("intervalStartInput");
const intervalEndInput = document.getElementById("intervalEndInput");
const intervalDeltaBadge = document.getElementById("intervalDeltaBadge");
const openSettingsButton = document.getElementById("openSettingsButton");
const closeSettingsButton = document.getElementById("closeSettingsButton");
const settingsView = document.getElementById("settingsView");
const settingsStatus = document.getElementById("settingsStatus");
const paramTabButton = document.getElementById("paramTabButton");
const structureTabButton = document.getElementById("structureTabButton");
const importTabButton = document.getElementById("importTabButton");
const runSearchButton = document.getElementById("runSearchButton");
const stopSearchButton = document.getElementById("stopSearchButton");
const searchProgressLabel = document.getElementById("searchProgressLabel");
const searchProgressPercent = document.getElementById("searchProgressPercent");
const searchProgressBar = document.getElementById("searchProgressBar");
const searchProgressDetail = document.getElementById("searchProgressDetail");
const launchModeSelect = document.getElementById("launchModeSelect");
const searchStartVxInput = document.getElementById("searchStartVxInput");
const searchSlimeBlockXInput = document.getElementById("searchSlimeBlockXInput");
const structureGrid = document.getElementById("structureGrid");
const loadSelectedStructureButton = document.getElementById("loadSelectedStructureButton");
const addCellButton = document.getElementById("addCellButton");
const removeCellButton = document.getElementById("removeCellButton");
const extendSelectionButton = document.getElementById("extendSelectionButton");
const selectAccelerationButton = document.getElementById("selectAccelerationButton");
const selectCycleButton = document.getElementById("selectCycleButton");
const markCycleButton = document.getElementById("markCycleButton");
const simulateStructureButton = document.getElementById("simulateStructureButton");
const structureTicksInput = document.getElementById("structureTicksInput");
const extendTotalLengthInput = document.getElementById("extendTotalLengthInput");
const cellWaterSelect = document.getElementById("cellWaterSelect");
const cellAmountInput = document.getElementById("cellAmountInput");
const cellFloorSelect = document.getElementById("cellFloorSelect");
const importLitematicButton = document.getElementById("importLitematicButton");
const litematicFileInput = document.getElementById("litematicFileInput");
const importReport = document.getElementById("importReport");
const exportLitematicButton = document.getElementById("exportLitematicButton");
const exportCycleRepeatInput = document.getElementById("exportCycleRepeatInput");
const targetSpeedInput = document.getElementById("targetSpeedInput");
const targetDwellInput = document.getElementById("targetDwellInput");
const maxThreadsInput = document.getElementById("maxThreadsInput");

refreshButton.addEventListener("click", () => loadRuns(true));
exportCsvButton.addEventListener("click", exportSelectedRunCsv);
openSettingsButton.addEventListener("click", () => setSettingsVisible(true));
closeSettingsButton.addEventListener("click", () => setSettingsVisible(false));
resetZoomButton.addEventListener("click", () => {
  resetView({ resetInterval: false });
  render();
});
selectModeButton.addEventListener("click", () => setInteractionMode(SELECT_MODE));
panModeButton.addEventListener("click", () => setInteractionMode(PAN_MODE));
zeroCoordButton.addEventListener("click", () => setCoordinateMode(ZERO_COORD_MODE));
rawCoordButton.addEventListener("click", () => setCoordinateMode(RAW_COORD_MODE));
preciseEndButton.addEventListener("click", () => setEndAlignmentMode(false));
truncatedEndButton.addEventListener("click", () => setEndAlignmentMode(true));
applyIntervalButton.addEventListener("click", applyIntervalFromInputs);
resetIntervalButton.addEventListener("click", resetIntervalToRunBounds);
paramTabButton.addEventListener("click", () => setSettingsTab("param"));
structureTabButton.addEventListener("click", () => setSettingsTab("structure"));
importTabButton.addEventListener("click", () => setSettingsTab("import"));
runSearchButton.addEventListener("click", runSearch);
stopSearchButton.addEventListener("click", stopSearch);
launchModeSelect.addEventListener("change", () => {
  syncLaunchDefaults();
});
targetSpeedInput.addEventListener("change", () => {
  syncTargetDwellDefault();
  syncCycleTargetLengthFromTicks();
});
loadSelectedStructureButton.addEventListener("click", loadSelectedRunIntoEditor);
addCellButton.addEventListener("click", addEditorCell);
removeCellButton.addEventListener("click", removeSelectedCells);
extendSelectionButton.addEventListener("click", extendSelectedCells);
selectAccelerationButton.addEventListener("click", selectAccelerationSegment);
selectCycleButton.addEventListener("click", selectCycleSegment);
markCycleButton.addEventListener("click", markSelectionAsCycle);
simulateStructureButton.addEventListener("click", simulateEditorStructure);
structureTicksInput.addEventListener("input", syncCycleTargetLengthFromTicks);
structureTicksInput.addEventListener("change", syncCycleTargetLengthFromTicks);
cellWaterSelect.addEventListener("change", applyInspectorToSelection);
cellAmountInput.addEventListener("change", applyInspectorToSelection);
cellFloorSelect.addEventListener("change", applyInspectorToSelection);
importLitematicButton.addEventListener("click", importLitematic);
exportLitematicButton.addEventListener("click", exportSelectedLitematic);

intervalStartInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") applyIntervalFromInputs();
});
intervalEndInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") applyIntervalFromInputs();
});

chartCanvas.addEventListener("wheel", onWheel, { passive: false });
chartCanvas.addEventListener("pointerdown", onPointerDown);
chartCanvas.addEventListener("pointermove", onPointerMove);
chartCanvas.addEventListener("pointerup", onPointerUp);
chartCanvas.addEventListener("pointercancel", onPointerCancel);
chartCanvas.addEventListener("pointerleave", onPointerLeave);
document.addEventListener("pointerup", finishStructureSelection);
document.addEventListener("pointercancel", finishStructureSelection);

const resizeObserver = new ResizeObserver(() => {
  resizeCanvas();
  render();
});
resizeObserver.observe(chartCanvas);

setInteractionMode(SELECT_MODE);
syncCoordinateModeUi();
syncEndAlignmentModeUi();
resizeCanvas();
initEditor();
syncHardwareDefaults();

function syncHardwareDefaults() {
  if (maxThreadsInput && !maxThreadsInput.dataset.initialized) {
    const cores = Math.max(1, Math.min(64, navigator.hardwareConcurrency || 4));
    maxThreadsInput.value = String(Math.min(8, cores));
    maxThreadsInput.max = String(cores);
    maxThreadsInput.dataset.initialized = "true";
  }
}

function setSettingsVisible(visible) {
  if (visible) rememberRunListScroll();
  settingsView.classList.toggle("active", visible);
  settingsView.setAttribute("aria-hidden", visible ? "false" : "true");
  if (visible) {
    if (!editorState.manualEditor) syncStructureEditorWithSelectedRun({ silent: true });
    renderSelectedStructurePanels();
  } else {
    renderRunList();
  }
}

function setSettingsTab(tab) {
  editorState.tab = tab;
  const buttons = {
    param: paramTabButton,
    structure: structureTabButton,
    import: importTabButton,
  };
  const pages = {
    param: document.getElementById("paramPage"),
    structure: document.getElementById("structurePage"),
    import: document.getElementById("importPage"),
  };
  for (const [name, button] of Object.entries(buttons)) {
    button.classList.toggle("active", name === tab);
    pages[name].classList.toggle("active", name === tab);
  }
  if (tab === "structure" && !editorState.manualEditor) syncStructureEditorWithSelectedRun({ silent: true });
  if (tab === "structure" || tab === "import") renderSelectedStructurePanels();
}

function setSettingsStatus(text) {
  settingsStatus.textContent = text;
}

function clearPreservedEditorContext() {
  editorState.launchSlimeBlockX = -1;
  editorState.loadedLaunchMode = "water";
  editorState.startEntityIdMod4 = 0;
  editorState.startInitialTickCount = 0;
  editorState.preservedStart = null;
  editorState.preservedLaunchConfig = null;
  editorState.preservedLaunch = null;
}

function markEditorAsManual({ preserveLaunchContext = false } = {}) {
  editorState.loadedRunId = null;
  editorState.manualEditor = true;
  if (!preserveLaunchContext) clearPreservedEditorContext();
}

function syncLaunchDefaults() {
  if (launchModeSelect.value === "piston") {
    searchStartVxInput.value = "0";
  } else {
    searchStartVxInput.value = "0";
  }
}

function syncTargetDwellDefault() {
  const speed = numberInput("targetSpeedInput", 0.5);
  if (speed > 0) {
    targetDwellInput.value = String(Math.max(1, Math.round(1 / speed)));
  }
}

function numberInput(id, fallback = 0) {
  const value = Number(document.getElementById(id).value);
  return Number.isFinite(value) ? value : fallback;
}

function intInput(id, fallback = 0) {
  const value = Number.parseInt(document.getElementById(id).value, 10);
  return Number.isFinite(value) ? value : fallback;
}

function cloneJson(value) {
  return value == null ? null : JSON.parse(JSON.stringify(value));
}

function defaultEditorStart() {
  return {
    x: 0.125,
    y: 0,
    vx: 0,
    vy: 0,
    startOnGround: true,
    entityIdMod4: 0,
    initialTickCount: 0,
  };
}

function resetVisibleStructureStartInputs() {
  const start = defaultEditorStart();
  document.getElementById("structureStartXInput").value = String(start.x);
  document.getElementById("structureStartYInput").value = String(start.y);
  document.getElementById("structureStartVxInput").value = String(start.vx);
  document.getElementById("structureStartVyInput").value = String(start.vy);
  document.getElementById("structureOnGroundInput").checked = start.startOnGround;
}

function setVisibleStructureStartInputs(start) {
  const value = start || defaultEditorStart();
  document.getElementById("structureStartXInput").value = String(value.x ?? 0.125);
  document.getElementById("structureStartYInput").value = String(value.y ?? 0);
  document.getElementById("structureStartVxInput").value = String(value.vx ?? 0);
  document.getElementById("structureStartVyInput").value = String(value.vy ?? 0);
  document.getElementById("structureOnGroundInput").checked = value.startOnGround !== false;
}

function hasPreservedEditorLaunchContext() {
  return editorState.preservedStart != null
    || editorState.preservedLaunchConfig != null
    || editorState.preservedLaunch != null;
}

function visibleEditorStartForLoadedRun(run, structure) {
  const launch = structure?.launch || {};
  if (
    run?.summary?.source === "game-storage"
    || (
      launch.mode === "water"
      && launch.rawStart
      && launch.effectiveStart
    )
  ) {
    return defaultEditorStart();
  }
  if (launch.mode === "piston") {
    return launch.rawStart || structure?.start || defaultEditorStart();
  }
  return structure?.start || defaultEditorStart();
}

var editorLaunchMode = function editorLaunchModeImpl() {
  if (hasPreservedEditorLaunchContext()) {
    return editorState.loadedLaunchMode === "piston" ? "piston" : "water";
  }
  return launchModeSelect.value === "piston" ? "piston" : "water";
};
window.editorLaunchMode = editorLaunchMode;

function getEditorPrefixLength() {
  return clamp(Math.round(Number(editorState.prefixLength) || 0), 0, editorState.cells.length);
}

function getEditorCycleLength() {
  return Math.max(0, editorState.cells.length - getEditorPrefixLength());
}

function syncCycleTargetLengthFromTicks() {
  if (!extendTotalLengthInput || !structureTicksInput) return;
  const cycleLength = getEditorCycleLength();
  if (cycleLength <= 0) return;
  const ticks = Math.max(1, intInput("structureTicksInput", 800));
  const targetSpeed = Math.max(0, numberInput("targetSpeedInput", 0.5));
  const estimatedCells = Math.ceil(ticks * targetSpeed) + 2;
  const targetLength = Math.max(editorState.cells.length, getEditorPrefixLength() + cycleLength, estimatedCells);
  extendTotalLengthInput.value = String(targetLength);
}

function initEditor() {
  editorState.cells = [
    makeEditorCell("forward", 8, "packed_ice"),
    makeEditorCell("forward", 7, "packed_ice"),
    makeEditorCell("forward", 6, "packed_ice"),
    makeEditorCell("dry", 0, "blue_ice"),
    makeEditorCell("dry", 0, "blue_ice"),
    makeEditorCell("dry", 0, "blue_ice"),
  ];
  editorState.prefixLength = 0;
  editorState.selectionStart = 0;
  editorState.selectionEnd = editorState.cells.length - 1;
  clearPreservedEditorContext();
  resetVisibleStructureStartInputs();
  syncInspectorFromSelection();
  syncCycleTargetLengthFromTicks();
  renderStructureEditor();
}

for (const inputId of ["structureOriginInput"]) {
  const input = document.getElementById(inputId);
  if (!input) continue;
  input.addEventListener("input", () => markEditorAsManual({ preserveLaunchContext: true }));
  input.addEventListener("change", () => markEditorAsManual({ preserveLaunchContext: true }));
}

for (const inputId of [
  "structureStartXInput",
  "structureStartYInput",
  "structureStartVxInput",
  "structureStartVyInput",
  "structureOnGroundInput",
]) {
  const input = document.getElementById(inputId);
  if (!input) continue;
  input.addEventListener("input", () => markEditorAsManual({ preserveLaunchContext: false }));
  input.addEventListener("change", () => markEditorAsManual({ preserveLaunchContext: false }));
}

function normalizeFloorChoice(floor) {
  return floor === "stone" || floor === "normal" ? "glass" : (floor || "glass");
}

function makeEditorCell(water = "dry", amount = 0, floor = "glass") {
  return { water, amount: Number(amount) || 0, floor: normalizeFloorChoice(floor) };
}

function editorCellToModelCell(cell) {
  const floor = normalizeFloorChoice(cell.floor);
  if (cell.water === "dry") {
    return { surface: null, amount: 0, flow: 0, floor, code: `D-${floor}` };
  }
  const amount = cell.water === "source" ? 8 : clamp(Math.round(Number(cell.amount) || 8), 1, 8);
  const flow = cell.water === "forward" ? 1 : (cell.water === "reverse" ? -1 : 0);
  const prefix = flow > 0 ? "F" : (flow < 0 ? "R" : "S");
  return { surface: amount / 9, amount, flow, floor, code: `${prefix}${amount}-${floor}` };
}

function waterKindFromModelCell(cell) {
  if (!cell || cell.surface == null || Number(cell.amount) === 0) return "dry";
  const code = String(cell.code || "").trim().toUpperCase();
  const codePrefix = code.match(/^[A-Z]+/)?.[0]?.charAt(0) ?? "";
  if (codePrefix === "F") return "forward";
  if (codePrefix === "R") return "reverse";
  if (codePrefix === "S") return "source";
  const flow = Number(cell.flow) || 0;
  if (flow > 0) return "forward";
  if (flow < 0) return "reverse";
  return "source";
}

function modelCellToEditorCell(cell) {
  if (!cell || cell.surface == null || Number(cell.amount) === 0) {
    return makeEditorCell("dry", 0, cell?.floor || "glass");
  }
  const amount = Number(cell.amount) || Math.round(Number(cell.surface) * 9) || 8;
  return makeEditorCell(waterKindFromModelCell(cell), amount, cell.floor || "glass");
}

function selectedRunStructure() {
  const run = getSelectedRun();
  if (!run?.structure) return null;
  return run.structure;
}

function selectedStructureCells(structure = selectedRunStructure()) {
  if (!structure) return [];
  return [
    ...(Array.isArray(structure.prefix) ? structure.prefix : []),
    ...(Array.isArray(structure.cycle) ? structure.cycle : []),
  ];
}

function structureCodeLabel(structure = selectedRunStructure()) {
  const run = getSelectedRun();
  return run?.summary?.structure || structure?.name || "-";
}

function renderSelectedStructurePanels() {
  renderSelectedStructurePanel("projection");
}

function renderSelectedStructurePanel(kind) {
  const badge = document.getElementById(kind === "projection" ? "projectionStructureBadge" : "selectedStructureBadge");
  const info = document.getElementById(kind === "projection" ? "projectionStructureInfo" : "selectedStructureInfo");
  const preview = document.getElementById(kind === "projection" ? "projectionStructurePreview" : "selectedStructurePreview");
  if (!badge || !info || !preview) return;

  const run = getSelectedRun();
  const structure = selectedRunStructure();
  info.innerHTML = "";
  preview.innerHTML = "";

  if (!run || !structure) {
    badge.textContent = "无结构";
    appendDetailRows(info, [["结果", run ? "当前结果没有结构数据" : "尚未选择结果"]]);
    return;
  }

  const prefix = Array.isArray(structure.prefix) ? structure.prefix : [];
  const cycle = Array.isArray(structure.cycle) ? structure.cycle : [];
  const cells = [...prefix, ...cycle];
  badge.textContent = `${prefix.length}+${cycle.length} 格`;
  appendDetailRows(info, [
    ["样本", getRunDisplayLabel(run)],
    ["编码", structureCodeLabel(structure)],
    ["加速段/循环", `${prefix.length} / ${cycle.length}`],
  ]);
  renderStructurePreview(preview, cells, prefix.length);
}

function renderStructurePreview(container, cells, prefixLength) {
  const maxCells = 96;
  cells.slice(0, maxCells).forEach((cell, index) => {
    const editorCell = modelCellToEditorCell(cell);
    const floor = normalizeFloorChoice(editorCell.floor);
    const tile = document.createElement("span");
    tile.className = "mini-structure-cell";
    if (index >= prefixLength) tile.classList.add("cycle-cell");
    tile.title = `${index < prefixLength ? "加速段" : "循环"} ${index}: ${describeWater(editorCell)} / ${floor}`;
    tile.innerHTML = `
      <span class="mini-water ${editorCell.water}">
        ${editorCell.water === "dry" ? "" : `<span style="height:${clamp((Number(editorCell.amount) || 8) / 8, 0.16, 1) * 100}%"></span>`}
      </span>
      <span class="mini-floor" style="background-image:url('${floorTexture(floor)}')"></span>
    `;
    container.appendChild(tile);
  });
  if (cells.length > maxCells) {
    const more = document.createElement("span");
    more.className = "mini-structure-more";
    more.textContent = `+${cells.length - maxCells}`;
    container.appendChild(more);
  }
}

function loadSelectedRunIntoEditor() {
  editorState.manualEditor = false;
  syncStructureEditorWithSelectedRun({ silent: false, force: true });
}

function syncStructureEditorWithSelectedRun({ silent = true, force = false } = {}) {
  if (!force && editorState.manualEditor) {
    renderStructureEditor();
    renderSelectedStructurePanels();
    return;
  }
  const run = getSelectedRun();
  const structure = selectedRunStructure();
  if (!run || !structure) {
    if (!silent) setSettingsStatus(run ? "当前结果没有可载入的结构数据" : "请先在结果列表选择一个样本");
    renderSelectedStructurePanels();
    return;
  }
  if (!force && editorState.loadedRunId === run.run_id) {
    renderStructureEditor();
    return;
  }
  const cells = selectedStructureCells(structure);
  if (cells.length === 0) {
    if (!silent) setSettingsStatus("当前结果结构为空");
    return;
  }
  editorState.cells = cells.map(modelCellToEditorCell);
  editorState.loadedRunId = run.run_id;
  editorState.manualEditor = false;
  const launch = structure.launch || {};
  const effectiveStart = launch.effectiveStart || structure.start || {};
  editorState.launchSlimeBlockX = Number(
    structure.launchConfig?.slimeBlockX ?? launch.slimeBlockX ?? -1,
  );
  editorState.loadedLaunchMode =
    structure.launchConfig?.mode === "piston" || launch.mode === "piston" || run.summary?.launch_mode === "piston"
      ? "piston"
      : "water";
  editorState.preservedStart = cloneJson(effectiveStart);
  editorState.preservedLaunchConfig = cloneJson(structure.launchConfig || null);
  editorState.preservedLaunch = cloneJson(structure.launch || null);
  editorState.prefixLength = clamp(Array.isArray(structure.prefix) ? structure.prefix.length : 0, 0, editorState.cells.length);
  if (getEditorCycleLength() > 0) {
    editorState.selectionStart = editorState.prefixLength;
    editorState.selectionEnd = editorState.cells.length - 1;
  } else {
    editorState.selectionStart = 0;
    editorState.selectionEnd = editorState.cells.length - 1;
  }
  document.getElementById("structureOriginInput").value = String(structure.originX ?? 0);
  const visibleStart = visibleEditorStartForLoadedRun(run, structure);
  setVisibleStructureStartInputs(visibleStart);
  editorState.startEntityIdMod4 = Number.isFinite(Number(visibleStart.entityIdMod4)) ? Number(visibleStart.entityIdMod4) : 0;
  editorState.startInitialTickCount = Number.isFinite(Number(visibleStart.initialTickCount)) ? Number(visibleStart.initialTickCount) : 0;
  syncInspectorFromSelection();
  syncCycleTargetLengthFromTicks();
  if (!silent) {
    setSettingsStatus(`已载入 ${getRunDisplayLabel(run)}：加速段 ${structure.prefix?.length || 0} 格，循环 ${structure.cycle?.length || 0} 格`);
  }
  renderStructureEditor();
  renderSelectedStructurePanels();
}

function getSelectionRange() {
  const start = Math.min(editorState.selectionStart ?? 0, editorState.selectionEnd ?? 0);
  const end = Math.max(editorState.selectionStart ?? 0, editorState.selectionEnd ?? 0);
  return { start, end };
}

function isCellSelected(index) {
  const range = getSelectionRange();
  return index >= range.start && index <= range.end;
}

function renderStructureEditor() {
  if (!structureGrid) return;
  structureGrid.innerHTML = "";
  editorState.cells.forEach((cell, index) => {
    const tile = document.createElement("button");
    tile.type = "button";
    tile.className = "structure-cell";
    tile.dataset.index = String(index);
    tile.classList.add(index < getEditorPrefixLength() ? "acceleration-cell" : "cycle-cell");
    if (isCellSelected(index)) tile.classList.add("selected");
    const floor = normalizeFloorChoice(cell.floor);
    const waterLabel = describeWater(cell);
    const waterFillHeight = cell.water === "dry" ? 0 : clamp((Number(cell.amount) || 8) / 8, 0.16, 1) * 100;
    const segmentLabel = index < getEditorPrefixLength() ? "加速段" : "循环";
    const segmentIndex = index < getEditorPrefixLength() ? index : index - getEditorPrefixLength();
    tile.innerHTML = `
      <span class="cell-index">${segmentLabel} ${segmentIndex}</span>
      <span class="cell-stack" aria-hidden="true">
        <span class="cell-layer water-layer ${cell.water}">
          ${
            cell.water === "dry"
              ? '<span class="water-empty">空气</span>'
              : `<span class="water-fill" style="height:${waterFillHeight}%; background-image:url('${assetTexture(cell.water === "source" ? "water_still" : "water_flow")}')"></span>
                 <span class="water-arrow">${cell.water === "reverse" ? "←" : (cell.water === "forward" ? "→" : "≈")}</span>`
          }
        </span>
        <span class="cell-layer floor-layer" style="background-image:url('${floorTexture(floor)}')"></span>
      </span>
      <span class="cell-caption">${escapeHtml(waterLabel)}</span>
      <span class="cell-floor">${escapeHtml(floor)}</span>
    `;
    tile.addEventListener("pointerdown", (event) => {
      event.preventDefault();
      if (event.shiftKey && editorState.selectionStart != null) {
        editorState.selectionEnd = index;
      } else {
        editorState.selectionStart = index;
        editorState.selectionEnd = index;
      }
      editorState.dragSelecting = true;
      syncInspectorFromSelection();
      renderStructureEditor();
    });
    tile.addEventListener("pointerenter", () => {
      if (!editorState.dragSelecting) return;
      editorState.selectionEnd = index;
      syncInspectorFromSelection();
      renderStructureEditor();
    });
    structureGrid.appendChild(tile);
  });
}

function finishStructureSelection() {
  if (!editorState.dragSelecting) return;
  editorState.dragSelecting = false;
  syncInspectorFromSelection();
  renderStructureEditor();
}

function assetTexture(name) {
  return `/mc-assets/block/${name}.png`;
}

function floorTexture(floor) {
  const texture = {
    stone: "stone",
    normal: "stone",
    glass: "glass",
    packed_ice: "packed_ice",
    blue_ice: "blue_ice",
    slime: "slime_block",
  }[floor] || "stone";
  return assetTexture(texture);
}

function describeWater(cell) {
  if (cell.water === "source") return `水源 水量${cell.amount || 8}`;
  if (cell.water === "forward") return `顺流 水量${cell.amount || 8}`;
  if (cell.water === "reverse") return `逆流 水量${cell.amount || 8}`;
  return "无水";
}

function syncInspectorFromSelection() {
  const cell = editorState.cells[editorState.selectionStart ?? 0];
  if (!cell) return;
  cellWaterSelect.value = cell.water;
  cellAmountInput.value = String(cell.amount || 8);
  cellFloorSelect.value = normalizeFloorChoice(cell.floor);
}

function applyInspectorToSelection() {
  const range = getSelectionRange();
  for (let index = range.start; index <= range.end; index += 1) {
    if (!editorState.cells[index]) continue;
    editorState.cells[index] = makeEditorCell(
      cellWaterSelect.value,
      cellWaterSelect.value === "dry" ? 0 : intInput("cellAmountInput", 8),
      cellFloorSelect.value,
    );
  }
  markEditorAsManual({ preserveLaunchContext: true });
  renderStructureEditor();
}

function addEditorCell() {
  const insertAt = (editorState.selectionEnd ?? editorState.cells.length - 1) + 1;
  const previousPrefixLength = getEditorPrefixLength();
  editorState.cells.splice(insertAt, 0, makeEditorCell(cellWaterSelect.value, intInput("cellAmountInput", 8), cellFloorSelect.value));
  if (insertAt < previousPrefixLength) editorState.prefixLength = previousPrefixLength + 1;
  markEditorAsManual({ preserveLaunchContext: true });
  editorState.selectionStart = insertAt;
  editorState.selectionEnd = insertAt;
  syncCycleTargetLengthFromTicks();
  renderStructureEditor();
}

function removeSelectedCells() {
  if (editorState.cells.length <= 1) return;
  const range = getSelectionRange();
  const previousPrefixLength = getEditorPrefixLength();
  editorState.cells.splice(range.start, range.end - range.start + 1);
  if (editorState.cells.length === 0) editorState.cells.push(makeEditorCell());
  const removedBeforeBoundary = Math.max(0, Math.min(range.end + 1, previousPrefixLength) - range.start);
  editorState.prefixLength = clamp(previousPrefixLength - removedBeforeBoundary, 0, editorState.cells.length);
  markEditorAsManual({ preserveLaunchContext: true });
  editorState.selectionStart = clamp(range.start, 0, editorState.cells.length - 1);
  editorState.selectionEnd = editorState.selectionStart;
  syncInspectorFromSelection();
  syncCycleTargetLengthFromTicks();
  renderStructureEditor();
}

function selectAccelerationSegment() {
  if (editorState.cells.length === 0) return;
  const prefixLength = getEditorPrefixLength();
  if (prefixLength <= 0) {
    editorState.selectionStart = 0;
    editorState.selectionEnd = 0;
    setSettingsStatus("当前结构没有加速段；可先框选一段，再设选区为循环段");
    syncInspectorFromSelection();
    renderStructureEditor();
    return;
  }
  editorState.selectionStart = 0;
  editorState.selectionEnd = Math.max(0, prefixLength - 1);
  syncInspectorFromSelection();
  renderStructureEditor();
}

function selectCycleSegment() {
  if (editorState.cells.length === 0) return;
  const prefixLength = getEditorPrefixLength();
  editorState.selectionStart = Math.min(prefixLength, editorState.cells.length - 1);
  editorState.selectionEnd = editorState.cells.length - 1;
  syncInspectorFromSelection();
  renderStructureEditor();
}

function markSelectionAsCycle() {
  if (editorState.cells.length === 0) return;
  const range = getSelectionRange();
  editorState.prefixLength = clamp(range.start, 0, editorState.cells.length - 1);
  editorState.selectionStart = editorState.prefixLength;
  editorState.selectionEnd = editorState.cells.length - 1;
  markEditorAsManual({ preserveLaunchContext: true });
  syncInspectorFromSelection();
  syncCycleTargetLengthFromTicks();
  setSettingsStatus(`已将第 ${editorState.prefixLength} 格起设为循环段`);
  renderStructureEditor();
}

function extendSelectedCells() {
  const range = getSelectionRange();
  const repeat = Math.max(1, intInput("repeatCountInput", 10));
  const targetLength = Math.max(0, intInput("extendTotalLengthInput", 0));
  const previousPrefixLength = getEditorPrefixLength();
  const selected = editorState.cells.slice(range.start, range.end + 1).map((cell) => ({ ...cell }));
  if (selected.length === 0) return;
  const insertion = [];
  if (targetLength > editorState.cells.length) {
    const needed = targetLength - editorState.cells.length;
    while (insertion.length < needed) {
      for (const cell of selected) {
        if (insertion.length >= needed) break;
        insertion.push({ ...cell });
      }
    }
  } else {
    for (let i = 0; i < repeat; i += 1) {
      insertion.push(...selected.map((cell) => ({ ...cell })));
    }
  }
  if (insertion.length === 0) return;
  editorState.cells.splice(range.end + 1, 0, ...insertion);
  if (range.end < previousPrefixLength) editorState.prefixLength = previousPrefixLength + insertion.length;
  markEditorAsManual({ preserveLaunchContext: true });
  editorState.selectionStart = range.end + 1;
  editorState.selectionEnd = range.end + insertion.length;
  syncCycleTargetLengthFromTicks();
  setSettingsStatus(`已延长 ${insertion.length} 格，总长度 ${editorState.cells.length}`);
  renderStructureEditor();
}

function structureFromEditor() {
  const prefixLength = getEditorPrefixLength();
  const launchMode = editorLaunchMode();
  const preservedStart = cloneJson(editorState.preservedStart);
  const visibleStart = {
    x: numberInput("structureStartXInput", 0.125),
    y: numberInput("structureStartYInput", 0),
    vx: numberInput("structureStartVxInput", 0),
    vy: numberInput("structureStartVyInput", 0),
    startOnGround: document.getElementById("structureOnGroundInput").checked,
    entityIdMod4: editorState.startEntityIdMod4 || 0,
    initialTickCount: editorState.startInitialTickCount || 0,
  };
  const structure = {
    name: "web-edited-structure",
    originX: numberInput("structureOriginInput", 0),
    start: launchMode === "piston" ? visibleStart : (preservedStart || visibleStart),
    prefix: editorState.cells.slice(0, prefixLength).map(editorCellToModelCell),
    cycle: editorState.cells.slice(prefixLength).map(editorCellToModelCell),
  };
  if (editorState.preservedLaunchConfig && launchMode !== "piston") {
    structure.launchConfig = cloneJson(editorState.preservedLaunchConfig);
  }
  if (editorState.preservedLaunch && launchMode !== "piston") {
    structure.launch = cloneJson(editorState.preservedLaunch);
  } else if (launchMode === "piston") {
    structure.launchConfig = {
      mode: "piston",
      slimeBlockX: numberInput("searchSlimeBlockXInput", editorState.launchSlimeBlockX),
    };
  } else {
    structure.launchConfig = {
      slimeBlockX: numberInput("searchSlimeBlockXInput", -1),
    };
  }
  return structure;
}

async function simulateEditorStructure() {
  setSettingsStatus("正在模拟结构...");
  try {
    const launchMode = editorLaunchMode();
    const payload = {
      structure: structureFromEditor(),
      options: {
        ticks: intInput("structureTicksInput", 800),
        targetSpeed: numberInput("targetSpeedInput", 0.5),
        targetDwellTicks: intInput("targetDwellInput", 2),
        launchMode,
        label: "结构模拟",
      },
    };
    const result = await postJson("/api/model/simulate", payload);
    await loadRuns(false);
    const launchDebug = result.run?.summary?.launch_mode || launchMode;
    setSettingsStatus(`Saved ${result.run.display_label} [launch=${launchDebug}]`);
  } catch (error) {
    console.error(error);
    setSettingsStatus(`模拟失败：${error.message}`);
  }
}

function buildSearchRequest() {
  return {
    params: {
      startX: numberInput("searchStartXInput", 0.125),
      startY: numberInput("searchStartYInput", 0),
      startVX: numberInput("searchStartVxInput", 0),
      launchMode: launchModeSelect.value,
      slimeBlockX: numberInput("searchSlimeBlockXInput", -1),
      targetSpeed: numberInput("targetSpeedInput", 0.5),
      minHitRate: numberInput("minHitRateInput", 1),
      ticks: intInput("searchTicksInput", 800),
      maxPrefixCells: intInput("maxPrefixInput", 9),
      maxCycleCells: intInput("maxCycleInput", 0),
      maxThreads: intInput("maxThreadsInput", navigator.hardwareConcurrency || 4),
      keep: intInput("keepCandidatesInput", 8),
    },
    options: {
      targetSpeed: numberInput("targetSpeedInput", 0.5),
      targetDwellTicks: intInput("targetDwellInput", 2),
    },
  };
}

function setSearchRunning(running) {
  state.searchTask.running = running;
  runSearchButton.disabled = running;
  stopSearchButton.disabled = !running;
}

function stopSearchPolling() {
  if (state.searchTask.timer) {
    clearTimeout(state.searchTask.timer);
    state.searchTask.timer = null;
  }
}

function updateSearchProgress(task) {
  const progress = task?.progress || {};
  const percent = Number.isFinite(progress.percent)
    ? progress.percent
    : (progress.total ? Math.round((Number(progress.checked || 0) / Number(progress.total)) * 1000) / 10 : 0);
  const safePercent = clamp(percent, 0, 100);
  searchProgressBar.style.width = `${safePercent}%`;
  searchProgressPercent.textContent = `${formatNumber(safePercent, safePercent % 1 ? 1 : 0)}%`;
  searchProgressLabel.textContent = progress.message || statusLabelForTask(task?.status);

  const parts = [];
  if (progress.total) parts.push(`枚举 ${progress.checked || 0}/${progress.total}`);
  if (Number.isFinite(progress.candidate_count)) parts.push(`候选 ${progress.candidate_count}`);
  if (Number.isFinite(progress.passing_count)) parts.push(`达标 ${progress.passing_count}`);
  if (Number.isFinite(progress.expanded_states)) parts.push(`状态 ${progress.expanded_states}`);
  if (Number.isFinite(progress.bucket_count)) parts.push(`桶 ${progress.bucket_count}`);
  if (Number.isFinite(progress.parallel_workers)) parts.push(`线程 ${progress.parallel_workers}`);
  if (Number.isFinite(progress.unique_count)) parts.push(`合并后 ${progress.unique_count}`);
  if (progress.write_total) parts.push(`写入 ${progress.written || 0}/${progress.write_total}`);
  if (task?.status === "cancelled") parts.push("已停止");
  if (task?.status === "failed" && task.error) parts.push(task.error);
  searchProgressDetail.textContent = parts.length ? parts.join(" · ") : "设置参数后开始穷举。";
}

function statusLabelForTask(status) {
  return {
    queued: "等待开始",
    running: "正在穷举",
    completed: "穷举完成",
    cancelled: "已停止穷举",
    failed: "穷举失败",
  }[status] || "等待开始";
}

async function pollSearchTask(taskId) {
  try {
    const response = await fetch(`/api/model/search/${taskId}`, { cache: "no-store" });
    const payload = await response.json();
    if (!response.ok || payload.ok === false) {
      throw new Error(payload.detail || payload.error || "查询任务失败");
    }
    const task = payload.task;
    updateSearchProgress(task);
    if (task.status === "completed") {
      stopSearchPolling();
      setSearchRunning(false);
      const result = task.result || {};
      const createdCount = result.created?.length || 0;
      const statusPrefix = `候选 ${result.candidate_count || 0} 个，达标 ${result.passing_count || 0} 个，合并后 ${result.unique_count || 0} 个，写入 ${createdCount} 个 run`;
      setSettingsStatus(createdCount > 0 ? statusPrefix : `${statusPrefix}；${result.message || "没有候选达到当前指标要求"}`);
      await loadRuns(false);
      return;
    }
    if (task.status === "cancelled") {
      stopSearchPolling();
      setSearchRunning(false);
      setSettingsStatus("已停止穷举");
      await loadRuns(true);
      return;
    }
    if (task.status === "failed") {
      stopSearchPolling();
      setSearchRunning(false);
      setSettingsStatus(`穷举失败：${task.error || "未知错误"}`);
      return;
    }
    state.searchTask.timer = setTimeout(() => pollSearchTask(taskId), 500);
  } catch (error) {
    console.error(error);
    stopSearchPolling();
    setSearchRunning(false);
    setSettingsStatus(`穷举状态查询失败：${error.message}`);
  }
}

async function runSearch() {
  if (state.searchTask.running) return;
  stopSearchPolling();
  setSettingsStatus("正在启动穷举...");
  updateSearchProgress({ status: "queued", progress: { message: "正在启动穷举...", percent: 0 } });
  setSearchRunning(true);
  try {
    const result = await postJson("/api/model/search", buildSearchRequest());
    const task = result.task;
    state.searchTask.id = task.task_id;
    updateSearchProgress(task);
    await pollSearchTask(task.task_id);
  } catch (error) {
    console.error(error);
    stopSearchPolling();
    setSearchRunning(false);
    setSettingsStatus(`穷举失败：${error.message}`);
  }
}

async function stopSearch() {
  if (!state.searchTask.id || !state.searchTask.running) return;
  setSettingsStatus("已请求停止，当前候选计算完成后会停下...");
  stopSearchButton.disabled = true;
  try {
    const payload = await postJson(`/api/model/search/${state.searchTask.id}/cancel`, {});
    updateSearchProgress(payload.task);
    if (payload.task?.status === "cancelled") {
      stopSearchPolling();
      setSearchRunning(false);
    }
  } catch (error) {
    console.error(error);
    stopSearchButton.disabled = false;
    setSettingsStatus(`停止失败：${error.message}`);
  }
}

async function importLitematic() {
  const file = litematicFileInput.files?.[0];
  if (!file) {
    setSettingsStatus("请选择 .litematic 文件");
    return;
  }
  setSettingsStatus("正在解析 Litematica...");
  try {
    const query = new URLSearchParams({
      floorY: String(intInput("litematicFloorYInput", 0)),
      fluidY: String(intInput("litematicFluidYInput", 1)),
      z: String(intInput("litematicZInput", 0)),
    });
    const response = await fetch(`/api/litematic/import?${query}`, {
      method: "POST",
      headers: { "Content-Type": "application/octet-stream" },
      body: await file.arrayBuffer(),
    });
    const payload = await response.json();
    if (!response.ok || payload.ok === false) {
      throw new Error(payload.detail || payload.error || "导入失败");
    }
    editorState.cells = payload.structure.prefix.map(modelCellToEditorCell);
    editorState.prefixLength = editorState.cells.length;
    markEditorAsManual();
    clearPreservedEditorContext();
    resetVisibleStructureStartInputs();
    editorState.selectionStart = 0;
    editorState.selectionEnd = Math.max(0, editorState.cells.length - 1);
    syncInspectorFromSelection();
    importReport.textContent = JSON.stringify({
      region: payload.region,
      cells: editorState.cells.length,
      unknownBlocks: payload.unknownBlocks,
    }, null, 2);
    setSettingsStatus("已导入到结构编辑器");
    setSettingsTab("structure");
    syncCycleTargetLengthFromTicks();
    renderStructureEditor();
  } catch (error) {
    console.error(error);
    setSettingsStatus(`导入失败：${error.message}`);
  }
}

async function exportSelectedLitematic() {
  const run = getSelectedRun();
  const structure = selectedRunStructure();
  if (!run || !structure) {
    setSettingsStatus(run ? "当前结果没有可导出的结构数据" : "请先在结果列表选择一个样本");
    renderSelectedStructurePanels();
    return;
  }
  setSettingsStatus("正在导出 Litematica...");
  try {
    const response = await fetch("/api/litematic/export", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        runId: run.run_id,
        structure,
        options: {
          cycleRepeat: Math.max(1, intInput("exportCycleRepeatInput", 64)),
          name: getRunDisplayLabel(run),
        },
      }),
    });
    if (!response.ok) {
      let message = "导出失败";
      try {
        const payload = await response.json();
        message = payload.detail || payload.error || message;
      } catch {
        message = await response.text();
      }
      throw new Error(message);
    }
    const blob = await response.blob();
    const disposition = response.headers.get("Content-Disposition") || "";
    const match = disposition.match(/filename="?([^"]+)"?/i);
    const filename = match?.[1] || `${safeFileStem(getRunDisplayLabel(run))}.litematic`;
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
    setSettingsStatus(`已导出 ${filename}`);
  } catch (error) {
    console.error(error);
    setSettingsStatus(`导出失败：${error.message}`);
  }
}

function exportSelectedRunCsv() {
  const run = getSelectedRun();
  if (!run) {
    setStatus("请先选择一个测试结果");
    return;
  }
  const rows = [];
  rows.push(["summary_key", "summary_value"]);
  for (const [key, value] of Object.entries(run.summary || {})) {
    if (value != null && typeof value === "object") {
      rows.push([key, JSON.stringify(value)]);
    } else {
      rows.push([key, value ?? ""]);
    }
  }
  rows.push([]);
  rows.push([
    "tick_index",
    "x",
    "x_raw",
    "y",
    "speed",
    "derived_speed",
    "vy",
    "floor",
    "on_ground",
    "log_time",
    "captured_at",
    "raw_line",
  ]);
  for (const point of run.points || []) {
    rows.push([
      point.tick_index ?? "",
      point.x ?? "",
      point.x_raw ?? "",
      point.y ?? "",
      point.speed ?? "",
      point.derived_speed ?? "",
      point.vy ?? "",
      point.floor ?? "",
      point.on_ground ?? "",
      point.log_time ?? "",
      point.captured_at ?? "",
      point.raw_line ?? "",
    ]);
  }
  const csv = rows.map(csvRow).join("\r\n");
  const blob = new Blob(["\ufeff", csv], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `${safeFileStem(getRunDisplayLabel(run))}.csv`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
  setStatus(`已导出 ${getRunDisplayLabel(run)} CSV`);
}

function csvRow(values) {
  return values.map((value) => {
    const text = String(value ?? "");
    if (/[",\r\n]/.test(text)) {
      return `"${text.replaceAll('"', '""')}"`;
    }
    return text;
  }).join(",");
}

async function postJson(url, payload) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  const data = await response.json();
  if (!response.ok || data.ok === false) {
    throw new Error(data.detail || data.error || "请求失败");
  }
  return data;
}

async function loadRuns(preserveSelection = false) {
  rememberRunListScroll();
  setStatus("正在刷新数据...");
  const response = await fetch("/api/runs", { cache: "no-store" });
  const payload = await response.json();
  state.payload = payload;

  const visibleRuns = getDisplayRuns().filter((run) => !run.summary.deleted);
  const selectedRun = resolveSelectedRun(visibleRuns, { preserveSelection });
  if (!selectedRun || selectedRun.run_id !== state.selectedRunId || getRunSelectionKey(selectedRun) !== state.selectedRunKey) {
    setSelectedRun(selectedRun, { resetPoint: true });
    state.selectedPointIndex = null;
    resetView({ resetInterval: true });
  }

  const activeRuns = visibleRuns.length;
  setStatus(`已加载 ${payload.run_count} 次测试，当前显示 ${activeRuns} 次，更新时间 ${payload.updated_at}`);
  renderRunList();
  updateSummary();
  renderSelectedStructurePanels();
  render();
}

function getSelectedRun() {
  if (!state.payload) return null;
  const runs = getDisplayRuns();
  return runs.find((run) => run.run_id === state.selectedRunId)
    ?? runs.find((run) => getRunSelectionKey(run) === state.selectedRunKey)
    ?? null;
}

function setSelectedRun(run, options = {}) {
  state.selectedRunId = run?.run_id ?? null;
  state.selectedRunKey = run ? getRunSelectionKey(run) : null;
  if (options.resetPoint !== false) state.selectedPointIndex = null;
}

function resolveSelectedRun(runs, { preserveSelection = false } = {}) {
  if (preserveSelection) {
    const byKey = runs.find((run) => getRunSelectionKey(run) === state.selectedRunKey);
    if (byKey) return byKey;
    const byId = runs.find((run) => run.run_id === state.selectedRunId);
    if (byId) return byId;
  }
  return runs.at(-1) ?? state.payload?.runs?.at(-1) ?? null;
}

function getRunSelectionKey(run) {
  if (!run) return null;
  return runEquivalenceKey(run) || `run:${run.run_id}`;
}

function rememberRunListScroll() {
  const runList = document.getElementById("runList");
  if (runList) state.runListScrollTop = runList.scrollTop;
}

function getRunsInDisplayOrder() {
  if (!state.payload) return [];
  return getDisplayRuns().sort((a, b) => a.run_id - b.run_id);
}

function getDisplayRuns() {
  if (!state.payload) return [];
  const groups = new Map();
  const result = [];
  for (const run of state.payload.runs) {
    const key = runEquivalenceKey(run);
    if (!key) {
      result.push(run);
      continue;
    }
    const existing = groups.get(key);
    if (!existing) {
      const copy = cloneRunForDisplay(run);
      groups.set(key, copy);
      result.push(copy);
      continue;
    }
    mergeEquivalentRun(existing, run);
  }
  return result;
}

function runEquivalenceKey(run) {
  if (!isSimulationRun(run)) return null;
  const deleteState = run.summary?.deleted ? "deleted" : "active";
  const motionKey = runMotionEquivalenceKey(run);
  if (motionKey) return `${deleteState}:points:${motionKey}`;
  if (run.summary?.equivalent_fingerprint) return `${deleteState}:fp:${run.summary.equivalent_fingerprint}`;
  return null;
}

function runMotionEquivalenceKey(run) {
  if (!Array.isArray(run.points) || run.points.length === 0) return null;
  return run.points.map((point) => [
    point.tick_index,
    stableRunNumber(point.x),
    stableRunNumber(point.y),
    stableRunNumber(point.speed),
    stableRunNumber(point.derived_speed),
    point.on_ground ? 1 : 0,
  ].join(",")).join(";");
}

function stableRunNumber(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return "";
  return number.toFixed(12);
}

function cloneRunForDisplay(run) {
  const structures = [...(run.summary?.structures || fallbackStructureList(run))];
  return {
    ...run,
    summary: {
      ...run.summary,
      structures,
      merged_run_ids: [...(run.summary?.merged_run_ids || [])],
      structure_count: structures.length,
    },
  };
}

function fallbackStructureList(run) {
  const structure = run.summary?.structure || run.structure?.name || run.display_label || run.label || `run ${run.run_id}`;
  return [{ structure, run_id: run.run_id }];
}

function mergeEquivalentRun(target, run) {
  const targetSummary = target.summary || {};
  const incoming = run.summary || {};
  const existingStructures = targetSummary.structures || fallbackStructureList(target);
  const incomingStructures = incoming.structures || fallbackStructureList(run);
  const seen = new Set(existingStructures.map((item) => item.structure));
  for (const item of incomingStructures) {
    if (!seen.has(item.structure)) {
      existingStructures.push({ ...item, run_id: item.run_id ?? run.run_id });
      seen.add(item.structure);
    }
  }
  targetSummary.structures = existingStructures;
  targetSummary.structure_count = existingStructures.length;
  targetSummary.merged_run_ids = [
    ...new Set([...(targetSummary.merged_run_ids || []), run.run_id, ...(incoming.merged_run_ids || [])]),
  ];
  target.summary = targetSummary;
}

function getRunActionIds(run) {
  return [...new Set([run.run_id, ...(run.summary?.merged_run_ids || [])])];
}

function getRunDisplayIndex(run) {
  if (!run) return null;
  if (Number.isFinite(run.display_index)) return run.display_index;
  const orderedRuns = getRunsInDisplayOrder();
  const index = orderedRuns.findIndex((item) => item.run_id === run.run_id);
  return index >= 0 ? index + 1 : null;
}

function getRunDisplayLabel(run) {
  if (!run) return "-";
  if (typeof run.display_label === "string" && run.display_label.trim()) {
    return run.display_label;
  }
  const displayIndex = getRunDisplayIndex(run);
  if (displayIndex == null) return run.label ?? "-";
  return `Run ${String(displayIndex).padStart(4, "0")}`;
}

function isSimulationRun(run) {
  return run?.summary?.source === "simulation" || run?.summary?.source === "reachability-search";
}

function isGameStorageRun(run) {
  return run?.summary?.source === "game-storage";
}

function isReadOnlyRun(run) {
  return isSimulationRun(run) || isGameStorageRun(run);
}

function getSourceBadge(run) {
  if (isSimulationRun(run)) return `<span class="run-badge">${"\u6a21\u578b\u6a21\u62df"}</span>`;
  if (isGameStorageRun(run)) return `<span class="run-badge">${"\u6e38\u620f\u5b9e\u6d4b"}</span>`;
  return "";
}

function getCurrentRunDwellHitRate(summary) {
  return summary?.steady_per_block_two_gt_dwell_hit_rate
    ?? summary?.per_block_two_gt_dwell_hit_rate
    ?? summary?.two_gt_hit_rate
    ?? null;
}

function getTargetDwellHitRate(summary) {
  return summary?.steady_per_block_target_dwell_hit_rate
    ?? summary?.per_block_target_dwell_hit_rate
    ?? getCurrentRunDwellHitRate(summary);
}

function getCurrentRunDwellHitRateXGt3(summary) {
  return summary?.per_block_two_gt_dwell_hit_rate_x_gt_3 ?? summary?.two_gt_hit_rate_x_gt_3 ?? null;
}

function getLongRunDwellHitRate(summary) {
  return summary?.long_per_block_two_gt_dwell_hit_rate ?? summary?.long_two_gt_hit_rate ?? null;
}

function getPreferredHitRate(summary) {
  return getTargetDwellHitRate(summary) ?? getLongRunDwellHitRate(summary) ?? getCurrentRunDwellHitRateXGt3(summary);
}

function numericOrNull(value) {
  if (value == null || value === "") return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function hasRecordedSteadyWindow(summary) {
  const source = summary?.steady_source;
  if (source && source !== "detected") return false;
  return Number.isFinite(numericOrNull(summary?.steady_start_tick))
    || Number.isFinite(numericOrNull(summary?.steady_start_raw_block))
    || Number.isFinite(numericOrNull(summary?.steady_start_block));
}

function getTargetSpeedForRun(run) {
  return Number.isFinite(run?.summary?.target_speed) ? Number(run.summary.target_speed) : 0.5;
}

function getTargetDwellTicksForRun(run) {
  const targetSpeed = getTargetSpeedForRun(run);
  return Math.max(1, Number(run?.summary?.target_dwell_ticks) || Math.round(1 / targetSpeed));
}

function getSteadyAverageSpeed(run) {
  const summary = run?.summary;
  const window = getSteadyWindow(run);
  if (window?.interval) {
    const computed = computeIntervalMetrics(run, window.interval.startX, window.interval.endX)?.avgSpeed;
    if (Number.isFinite(computed)) return computed;
  }
  if (Number.isFinite(summary?.steady_avg_speed)) return summary.steady_avg_speed;
  return null;
}

function getSteadyHitRate(run) {
  const summary = run?.summary;
  const window = getSteadyWindow(run);
  if (window?.interval) {
    const computed = computeIntervalMetrics(run, window.interval.startX, window.interval.endX)?.twoGtHitRate;
    if (Number.isFinite(computed)) return computed;
  }
  if (Number.isFinite(summary?.steady_per_block_target_dwell_hit_rate)) {
    return summary.steady_per_block_target_dwell_hit_rate;
  }
  if (hasRecordedSteadyWindow(summary)) {
    const summaryValue = summary?.per_block_target_dwell_hit_rate ?? summary?.per_block_two_gt_dwell_hit_rate;
    if (Number.isFinite(summaryValue)) return summaryValue;
  }
  return getTargetDwellHitRate(summary);
}

function clampUnit(value) {
  if (!Number.isFinite(value)) return null;
  return Math.min(1, Math.max(0, value));
}

function getRawOffset(run) {
  const summary = run?.summary;
  if (Number.isFinite(summary?.start_x_raw) && Number.isFinite(summary?.start_x)) {
    return summary.start_x_raw - summary.start_x;
  }
  const firstPoint = run?.points?.find((point) => Number.isFinite(point?.x_raw) && Number.isFinite(point?.x));
  return firstPoint ? firstPoint.x_raw - firstPoint.x : 0;
}

function zeroXToDisplayX(run, zeroX, coordinateMode = state.coordinateMode) {
  if (!Number.isFinite(zeroX)) return null;
  return coordinateMode === RAW_COORD_MODE ? zeroX + getRawOffset(run) : zeroX;
}

function displayXToZeroX(run, displayX, coordinateMode = state.coordinateMode) {
  if (!Number.isFinite(displayX)) return null;
  return coordinateMode === RAW_COORD_MODE ? displayX - getRawOffset(run) : displayX;
}

function getPointX(point) {
  if (!point) return null;
  const preferred = state.coordinateMode === RAW_COORD_MODE ? point.x_raw : point.x;
  const fallback = state.coordinateMode === RAW_COORD_MODE ? point.x : point.x_raw;
  return Number.isFinite(preferred) ? preferred : (Number.isFinite(fallback) ? fallback : null);
}

function getPointAlternateX(point) {
  if (!point) return null;
  const alternate = state.coordinateMode === RAW_COORD_MODE ? point.x : point.x_raw;
  return Number.isFinite(alternate) ? alternate : null;
}

function getSummaryStartX(run) {
  const summary = run?.summary;
  const preferred = state.coordinateMode === RAW_COORD_MODE ? summary?.start_x_raw : summary?.start_x;
  const fallback = state.coordinateMode === RAW_COORD_MODE ? summary?.start_x : summary?.start_x_raw;
  return Number.isFinite(preferred) ? preferred : (Number.isFinite(fallback) ? fallback : null);
}

function getSummaryEndX(run) {
  const summary = run?.summary;
  const preferred = state.coordinateMode === RAW_COORD_MODE ? summary?.end_x_raw : summary?.end_x;
  const fallback = state.coordinateMode === RAW_COORD_MODE ? summary?.end_x : summary?.end_x_raw;
  return Number.isFinite(preferred) ? preferred : (Number.isFinite(fallback) ? fallback : null);
}

function getPointRawX(point) {
  if (!point) return null;
  return Number.isFinite(point.x_raw) ? point.x_raw : (Number.isFinite(point.x) ? point.x : null);
}

function getZeroXForPoint(run, point) {
  if (!point) return null;
  if (Number.isFinite(point.x)) return point.x;
  if (Number.isFinite(point.x_raw)) return point.x_raw - getRawOffset(run);
  return null;
}

function findPointByTick(run, tick) {
  if (!run || !Number.isFinite(tick)) return null;
  return run.points.find((point) => Number(point.tick_index) >= Number(tick)) ?? null;
}

function findPointByBlock(run, block, startTick = null) {
  if (!run || !Number.isFinite(block)) return null;
  return run.points.find((point) => {
    if (Number.isFinite(startTick) && Number(point.tick_index) < Number(startTick)) return false;
    const rawX = Number.isFinite(point.x_raw) ? point.x_raw : point.x;
    return Number.isFinite(rawX) && Math.floor(rawX) >= Number(block);
  }) ?? null;
}

function findFirstStableDwellGroup(run, minTick = null, minRawBlock = null, minStableGroups = 12) {
  const targetDwellTicks = getTargetDwellTicksForRun(run);
  const groups = computeBlockDwellGroups(run?.points || []);
  const needed = Math.max(3, Math.min(minStableGroups, groups.length));
  for (let index = 0; index < groups.length; index += 1) {
    const group = groups[index];
    if (Number.isFinite(minTick) && Number(group[0].tick_index) < Number(minTick)) continue;
    if (Number.isFinite(minRawBlock) && Number(group[0].blockX) < Number(minRawBlock)) continue;
    let stableCount = 0;
    let previousBlock = null;
    for (let cursor = index; cursor < groups.length; cursor += 1) {
      const blockX = groups[cursor][0].blockX;
      if (previousBlock != null && blockX !== previousBlock + 1) break;
      if (groups[cursor].length !== targetDwellTicks) break;
      previousBlock = blockX;
      stableCount += 1;
      if (stableCount >= needed) return group;
    }
  }
  return null;
}

function getWindowEndPoint(run, endBlock) {
  if (!run?.points?.length) return null;
  if (!Number.isFinite(endBlock)) return run.points[run.points.length - 1];
  for (let index = run.points.length - 1; index >= 0; index -= 1) {
    const point = run.points[index];
    const rawX = Number.isFinite(point.x_raw) ? point.x_raw : point.x;
    if (Number.isFinite(rawX) && Math.floor(rawX) <= Number(endBlock)) return point;
  }
  return run.points[run.points.length - 1];
}

function cadenceWindowMetricsForRun(run, startIndex, pairCount, cadenceTicks, targetSpeed) {
  const points = run?.points || [];
  const endIndex = startIndex + pairCount * cadenceTicks;
  if (startIndex < 0 || endIndex >= points.length) return null;
  const targetDistance = targetSpeed * cadenceTicks;
  let blockHits = 0;
  let meanAbsDistanceError = 0;

  for (let index = 0; index < pairCount; index += 1) {
    const point0 = points[startIndex + index * cadenceTicks];
    const point1 = points[startIndex + (index + 1) * cadenceTicks];
    const x0 = getPointRawX(point0);
    const x1 = getPointRawX(point1);
    if (!Number.isFinite(x0) || !Number.isFinite(x1)) return null;
    const distance = x1 - x0;
    meanAbsDistanceError += Math.abs(distance - targetDistance);
    if (Math.floor(x1) - Math.floor(x0) === 1) blockHits += 1;
  }

  const startX = getPointRawX(points[startIndex]);
  const endX = getPointRawX(points[endIndex]);
  if (!Number.isFinite(startX) || !Number.isFinite(endX)) return null;
  return {
    point: points[startIndex],
    tick: Number(points[startIndex].tick_index),
    block: Math.floor(startX),
    blockHitRate: blockHits / pairCount,
    averageSpeed: (endX - startX) / (pairCount * cadenceTicks),
    meanAbsDistanceError: meanAbsDistanceError / pairCount,
  };
}

function detectSteadyWindowForRun(run) {
  const points = run?.points || [];
  if (points.length < 8) return null;
  const targetSpeed = Number(run.summary?.target_speed) || 0.5;
  const targetDwellTicks = Number(run.summary?.target_dwell_ticks) || Math.max(1, Math.round(1 / targetSpeed));
  const cadenceTicks = Math.max(1, Math.round(targetDwellTicks));
  let pairCount = Math.min(20, Math.floor((points.length - 1) / cadenceTicks) - 1);
  pairCount = Math.max(3, pairCount);
  if (pairCount <= 0 || pairCount * cadenceTicks >= points.length) return null;
  const tolerance = 0.05;
  const speedTolerance = 0.02;
  const blockHitThreshold = 0.98;

  for (let startIndex = 0; startIndex + pairCount * cadenceTicks < points.length; startIndex += 1) {
    const metrics = cadenceWindowMetricsForRun(run, startIndex, pairCount, cadenceTicks, targetSpeed);
    if (!metrics) continue;
    if (
      metrics.blockHitRate >= blockHitThreshold
      && metrics.meanAbsDistanceError <= tolerance
      && Math.abs(metrics.averageSpeed - targetSpeed) <= speedTolerance
    ) {
      const lastPoint = points[points.length - 1];
      const lastRawX = getPointRawX(lastPoint);
      const completeGroup = findFirstStableDwellGroup(run, metrics.tick, metrics.block, pairCount);
      if (!completeGroup) continue;
      const startPoint = completeGroup[0];
      const startRawX = getPointRawX(startPoint);
      return {
        source: "detected",
        startTick: Number.isFinite(startPoint?.tick_index) ? Number(startPoint.tick_index) : (Number.isFinite(metrics.tick) ? metrics.tick : startIndex),
        startBlock: Number.isFinite(startRawX) ? Math.floor(startRawX - getRawOffset(run)) : metrics.block,
        startRawBlock: Number.isFinite(startRawX) ? Math.floor(startRawX) : metrics.block,
        endBlock: Number.isFinite(lastRawX) ? Math.floor(lastRawX) : null,
        endDisplayBlock: Number.isFinite(lastRawX) ? Math.floor(lastRawX - getRawOffset(run)) : null,
        startPoint,
        endPoint: lastPoint,
      };
    }
  }
  return null;
}

function getSteadyWindow(run) {
  if (!run?.points?.length) return null;
  const summary = run.summary || {};
  const recordedSource = summary.steady_source;
  const allowRecordedWindow = !recordedSource || recordedSource === "detected";
  const startTick = allowRecordedWindow ? numericOrNull(summary.steady_start_tick) : null;
  const startRawBlock = allowRecordedWindow ? numericOrNull(summary.steady_start_raw_block) : null;
  const endRawBlock = allowRecordedWindow ? numericOrNull(summary.steady_end_raw_block) : null;
  const hasRecordedWindow = allowRecordedWindow && (Number.isFinite(startTick) || Number.isFinite(startRawBlock));
  const detectedWindow = hasRecordedWindow ? null : detectSteadyWindowForRun(run);
  if (!hasRecordedWindow && !detectedWindow) return null;

  const startPoint = detectedWindow?.startPoint
    ?? (Number.isFinite(startRawBlock) ? findPointByBlock(run, startRawBlock, startTick) : findPointByTick(run, startTick))
    ?? findPointByTick(run, startTick)
    ?? findPointByBlock(run, startRawBlock);
  const alignedGroup = detectedWindow ? null : findFirstStableDwellGroup(
    run,
    startPoint?.tick_index ?? startTick,
    Number.isFinite(startRawBlock) ? startRawBlock : null,
  );
  const alignedStartPoint = alignedGroup?.[0] ?? startPoint;
  const endPoint = detectedWindow?.endPoint ?? getWindowEndPoint(run, endRawBlock);
  const startZeroX = getZeroXForPoint(run, alignedStartPoint);
  const endZeroX = getZeroXForPoint(run, endPoint);
  const startX = zeroXToDisplayX(run, startZeroX);
  const endX = zeroXToDisplayX(run, endZeroX);
  const interval = normalizeInterval(startX, endX);
  const startRawX = getPointRawX(alignedStartPoint);
  return {
    source: hasRecordedWindow ? (summary.steady_source ?? null) : (detectedWindow?.source ?? null),
    startTick: Number.isFinite(alignedStartPoint?.tick_index) ? Number(alignedStartPoint.tick_index) : (Number.isFinite(startTick) ? startTick : (detectedWindow?.startTick ?? null)),
    startBlock: Number.isFinite(startZeroX) ? Math.floor(startZeroX) : (detectedWindow?.startBlock ?? null),
    startRawBlock: Number.isFinite(startRawX) ? Math.floor(startRawX) : (detectedWindow?.startRawBlock ?? (Number.isFinite(startRawBlock) ? startRawBlock : null)),
    startX,
    endBlock: Number.isFinite(endZeroX) ? Math.floor(endZeroX) : (detectedWindow?.endDisplayBlock ?? null),
    interval,
  };
}

function getSelectedPoint() {
  const run = getSelectedRun();
  if (!run || state.selectedPointIndex == null) return null;
  return run.points[state.selectedPointIndex] ?? null;
}

function getEffectiveInterval() {
  return normalizeInterval(
    state.previewInterval?.startX ?? state.interval.startX,
    state.previewInterval?.endX ?? state.interval.endX,
  );
}

function resizeCanvas() {
  const rect = chartCanvas.getBoundingClientRect();
  const width = Math.max(320, Math.round(rect.width || 1200));
  const height = Math.max(320, Math.round(rect.height || 680));
  const dpr = window.devicePixelRatio || 1;

  if (
    chartCanvas.width === Math.round(width * dpr) &&
    chartCanvas.height === Math.round(height * dpr) &&
    state.canvas.dpr === dpr
  ) {
    state.canvas.width = width;
    state.canvas.height = height;
    return;
  }

  chartCanvas.width = Math.round(width * dpr);
  chartCanvas.height = Math.round(height * dpr);
  state.canvas = { width, height, dpr };
}

function getPlotRect() {
  const width = state.canvas.width || chartCanvas.clientWidth || 1200;
  const height = state.canvas.height || chartCanvas.clientHeight || 680;
  return {
    left: PLOT_PADDING.left,
    right: width - PLOT_PADDING.right,
    top: PLOT_PADDING.top,
    bottom: height - PLOT_PADDING.bottom,
  };
}

function getRunBounds(run) {
  if (!run || run.points.length === 0) return null;
  const xValues = run.points.map((point) => getPointX(point)).filter((value) => Number.isFinite(value));
  const speedValues = run.points
    .map((point) => (point.speed != null ? point.speed : point.derived_speed))
    .filter((value) => Number.isFinite(value));
  if (xValues.length === 0 || speedValues.length === 0) return null;
  return {
    xMin: Math.min(...xValues),
    xMax: Math.max(...xValues),
    yMin: Math.min(...speedValues),
    yMax: Math.max(...speedValues),
  };
}

function getDefaultIntervalForRun(run) {
  const bounds = getRunBounds(run);
  if (!bounds) return null;
  const steadyWindow = getSteadyWindow(run);
  if (steadyWindow?.interval) {
    return normalizeInterval(
      clamp(steadyWindow.interval.startX, bounds.xMin, bounds.xMax),
      clamp(steadyWindow.interval.endX, bounds.xMin, bounds.xMax),
    );
  }
  return normalizeInterval(bounds.xMin, bounds.xMax);
}

function resetView(options = {}) {
  const { resetInterval = true } = options;
  const run = getSelectedRun();
  const bounds = getRunBounds(run);
  if (!bounds) {
    state.view = null;
    if (resetInterval) {
      state.interval.startX = null;
      state.interval.endX = null;
      state.previewInterval = null;
      syncIntervalInputs();
    }
    return;
  }

  const yMin = Math.min(0.42, bounds.yMin);
  const yMax = Math.max(0.58, bounds.yMax);
  state.view = {
    xMin: Math.max(bounds.xMin - 0.4, Math.min(0, bounds.xMin)),
    xMax: bounds.xMax + 0.4,
    yMin: yMin - 0.01,
    yMax: yMax + 0.01,
  };

  if (resetInterval) {
    const defaultInterval = getDefaultIntervalForRun(run);
    state.interval.startX = defaultInterval?.startX ?? bounds.xMin;
    state.interval.endX = defaultInterval?.endX ?? bounds.xMax;
    state.previewInterval = null;
    syncIntervalInputs();
  }
}

function resetIntervalToRunBounds() {
  const run = getSelectedRun();
  const defaultInterval = getDefaultIntervalForRun(run);
  if (!defaultInterval) return;
  state.interval.startX = defaultInterval.startX;
  state.interval.endX = defaultInterval.endX;
  state.previewInterval = null;
  syncIntervalInputs();
  updateSummary();
  render();
}

function renderRunList() {
  const runList = document.getElementById("runList");
  rememberRunListScroll();
  runList.innerHTML = "";
  if (!state.payload) return;

  const sortedRuns = getDisplayRuns().sort((a, b) => b.run_id - a.run_id);
  const selectedKey = state.selectedRunKey;
  for (const run of sortedRuns) {
    const item = document.createElement("div");
    item.className = "run-item";
    if (run.run_id === state.selectedRunId || getRunSelectionKey(run) === selectedKey) item.classList.add("active");
    if (run.summary.deleted) item.classList.add("deleted");
    if (isSimulationRun(run)) item.classList.add("simulation");

    const avgSpeed = formatNumber(getSteadyAverageSpeed(run), 6);
    const hitRate = formatPercent(getSteadyHitRate(run));
    const sourceBadge = getSourceBadge(run);
    item.innerHTML = `
      <div class="run-item-header">
        <div class="run-item-title">${getRunDisplayLabel(run)} ${sourceBadge}</div>
        <div>${run.summary.deleted ? "已删除" : ""}</div>
      </div>
      <div class="run-item-meta">
        <span>样本 ${run.summary.sample_count}</span>
        <span>x 末端 ${formatNumber(getSummaryEndX(run), 3)}</span>
        <span>稳态均速 ${avgSpeed}</span>
        <span>稳态命中 ${hitRate}</span>
      </div>
      <div class="run-actions"></div>
    `;

    item.addEventListener("click", (event) => {
      if (event.target.closest("button")) return;
      rememberRunListScroll();
      setSelectedRun(run, { resetPoint: true });
      resetView({ resetInterval: true });
      if (editorState.tab === "structure" && !editorState.manualEditor) {
        syncStructureEditorWithSelectedRun({ silent: true, force: true });
      }
      updateSummary();
      renderSelectedStructurePanels();
      renderRunList();
      render();
    });

    const actions = item.querySelector(".run-actions");
    const actionIds = getRunActionIds(run);
    if (run.summary.deleted) {
      const restoreButton = document.createElement("button");
      restoreButton.className = "ghost-button";
      restoreButton.type = "button";
      restoreButton.textContent = "恢复";
      restoreButton.addEventListener("click", async () => {
        await Promise.all(actionIds.map((runId) => fetch(`/api/runs/${runId}/restore`, { method: "POST" })));
        await loadRuns(true);
      });
      actions.appendChild(restoreButton);

      const purgeButton = document.createElement("button");
      purgeButton.className = "danger-button";
      purgeButton.type = "button";
      purgeButton.textContent = "彻底删除";
      purgeButton.addEventListener("click", async () => {
        await Promise.all(actionIds.map((runId) => fetch(`/api/runs/${runId}/purge`, { method: "POST" })));
        if (actionIds.includes(state.selectedRunId)) {
          setSelectedRun(null, { resetPoint: true });
        }
        await loadRuns(false);
      });
      actions.appendChild(purgeButton);
    } else {
      const deleteButton = document.createElement("button");
      deleteButton.className = "danger-button";
      deleteButton.type = "button";
      deleteButton.textContent = "删除";
      deleteButton.addEventListener("click", async () => {
        await Promise.all(actionIds.map((runId) => fetch(`/api/runs/${runId}`, { method: "DELETE" })));
        if (actionIds.includes(state.selectedRunId)) {
          setSelectedRun(null, { resetPoint: true });
        }
        await loadRuns(false);
      });
      actions.appendChild(deleteButton);
    }

    runList.appendChild(item);
  }
  runList.scrollTop = Math.min(state.runListScrollTop, Math.max(0, runList.scrollHeight - runList.clientHeight));
}

function updateSummary() {
  const run = getSelectedRun();
  if (run && (state.selectedRunId !== run.run_id || state.selectedRunKey !== getRunSelectionKey(run))) {
    setSelectedRun(run, { resetPoint: false });
  }
  const steadyWindow = getSteadyWindow(run);
  const steadyMetrics = run && steadyWindow?.interval
    ? computeIntervalMetrics(run, steadyWindow.interval.startX, steadyWindow.interval.endX)
    : null;
  const computedSteadyFailures =
    steadyMetrics && Number.isFinite(steadyMetrics.twoGtBlockCount) && Number.isFinite(steadyMetrics.twoGtHitCount)
      ? steadyMetrics.twoGtBlockCount - steadyMetrics.twoGtHitCount
      : null;
  const steadyDwellBlocks = steadyMetrics?.twoGtBlockCount ?? run?.summary?.steady_dwell_blocks ?? run?.summary?.dwell_blocks ?? null;
  const steadyDwellFailures = computedSteadyFailures ?? run?.summary?.steady_dwell_failures ?? run?.summary?.dwell_failures ?? null;
  const steadyEntryLabel = run && steadyWindow
    ? `${formatNumber(steadyWindow.startTick, 0)} gt / x=${formatNumber(steadyWindow.startX, 3)}`
    : "-";
  const metrics = {
    metricRun: run ? getRunDisplayLabel(run) : "-",
    metricAvgSpeed: run ? `${formatNumber(getSteadyAverageSpeed(run), 6)} m/gt` : "-",
    metricHitRate: run ? formatPercent(getSteadyHitRate(run)) : "-",
    metricSteadyEntry: steadyEntryLabel,
    metricSamples: run ? String(run.summary.sample_count) : "-",
  };
  for (const [id, value] of Object.entries(metrics)) {
    document.getElementById(id).textContent = value;
  }

  const runDetails = document.getElementById("runDetails");
  runDetails.innerHTML = "";
  if (run) {
    appendDetailRows(runDetails, [
      ["起止 x", `${formatNumber(getSummaryStartX(run), 3)} - ${formatNumber(getSummaryEndX(run), 3)}`],
      ["时长", `${run.summary.duration_gt} gt`],
    ]);
    appendDetailRows(runDetails, [
      ["稳态入口", steadyEntryLabel],
      ["稳态后平均速度", `${formatNumber(getSteadyAverageSpeed(run), 6)} m/gt`],
      ["稳态后驻留命中率", formatPercent(getSteadyHitRate(run))],
      ["稳态后方块数", Number.isFinite(steadyDwellBlocks) ? String(steadyDwellBlocks) : "-"],
      ["稳态后异常", Number.isFinite(steadyDwellFailures) ? String(steadyDwellFailures) : "-"],
    ]);
    if (isSimulationRun(run)) {
      const structures = Array.isArray(run.summary.structures) ? run.summary.structures : fallbackStructureList(run);
      appendDetailRows(runDetails, [
        ["来源", "\u6a21\u578b\u6a21\u62df"],
        ["结构", run.summary.structure ?? "-"],
        ["等价结构数", String(run.summary.structure_count ?? 1)],
      ]);
      appendEquivalentStructuresRow(runDetails, structures);
    }
  }

  const intervalDetails = document.getElementById("intervalDetails");
  intervalDetails.innerHTML = "";
  if (!run) {
    intervalDeltaBadge.textContent = "x变化量 -";
    appendDetailRows(intervalDetails, [["说明", "暂无可用区间"]]);
  } else {
    const interval = getEffectiveInterval();
    const intervalMetrics = interval ? computeIntervalMetrics(run, interval.startX, interval.endX) : null;
    if (!intervalMetrics) {
      intervalDeltaBadge.textContent = "x变化量 -";
      appendDetailRows(intervalDetails, [["说明", "拖拽图表或输入起止 x 来选择区间"]]);
    } else {
      intervalDeltaBadge.textContent = `x变化量 ${formatNumber(intervalMetrics.deltaX, 3)}`;
      appendDetailRows(intervalDetails, [
        ["平均速度", `${formatNumber(intervalMetrics.avgSpeed, 6)} m/gt`],
        ["样本数", String(intervalMetrics.sampleCount)],
        ["2gt方块数", String(intervalMetrics.twoGtBlockCount)],
        ["每格2gt驻留命中率", formatPercent(intervalMetrics.twoGtHitRate)],
        ["尾部未完整判定", String(intervalMetrics.tailTruncatedGroupCount ?? 0)],
        ["末端位置吻合率", formatPercent(intervalMetrics.endAlignmentRate)],
        ["末端累计偏移量", formatSignedNumber(intervalMetrics.endOffset, 6)],
        ["区间时长", `${formatNumber(intervalMetrics.durationGt, 3)} gt`],
      ]);
    }
  }

  const pointDetails = document.getElementById("pointDetails");
  pointDetails.innerHTML = "";
  const point = getSelectedPoint();
  if (point) {
    appendDetailRows(pointDetails, [
      ["tick", String(point.tick_index)],
      ["x", formatNumber(getPointX(point), 6)],
      ["记录速度", formatNumber(point.speed, 6)],
      ["推导速度", formatNumber(point.derived_speed, 6)],
      ["记录时间", point.log_time ?? "-"],
    ]);
  } else {
    appendDetailRows(pointDetails, [["说明", "点击折线点查看单点详情"]]);
  }
}

function appendEquivalentStructuresRow(container, structures) {
  const row = document.createElement("div");
  row.className = "detail-row";
  const count = structures.length;
  const label = count > 0 ? `展开 ${count} 条` : "-";
  const list = count > 0
    ? structures.map((item, index) => `${index + 1}. ${item.structure}`).join("\n")
    : "暂无";
  row.innerHTML = `
    <dt>等价结构</dt>
    <dd>
      <details>
        <summary>${escapeHtml(label)}</summary>
        <div class="equivalent-structures-list">${escapeHtml(list).replaceAll("\n", "<br>")}</div>
      </details>
    </dd>
  `;
  container.appendChild(row);
}

function appendDetailRows(container, rows) {
  for (const [key, value] of rows) {
    const row = document.createElement("div");
    row.className = "detail-row";
    row.innerHTML = `<dt>${escapeHtml(key)}</dt><dd>${escapeHtml(String(value))}</dd>`;
    container.appendChild(row);
  }
}

function render() {
  resizeCanvas();

  const ctx = chartContext;
  const { width, height, dpr } = state.canvas;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);
  state.pointCache = [];
  syncModeUi();

  const run = getSelectedRun();
  if (!run || run.points.length === 0) {
    ctx.fillStyle = "#5d6c7a";
    ctx.font = '18px "Segoe UI"';
    ctx.fillText("没有可显示的数据", 40, 60);
    return;
  }

  if (!state.view) resetView({ resetInterval: true });
  const plot = getPlotRect();
  const { xMin, xMax, yMin, yMax } = state.view;

  drawGrid(ctx, plot, xMin, xMax, yMin, yMax);
  drawAxisLabels(ctx, plot, xMin, xMax, yMin, yMax);
  drawIdealLine(ctx, plot, xMin, xMax, yMin, yMax, run);
  drawIntervalBand(ctx, plot, xMin, xMax, state.interval, "committed");
  if (state.previewInterval) {
    drawIntervalBand(ctx, plot, xMin, xMax, state.previewInterval, "preview");
  }

  const series = run.points
    .map((point, index) => ({
      index,
      x: getPointX(point),
      y: point.speed != null ? point.speed : point.derived_speed,
      log_time: point.log_time,
    }))
    .filter((point) => Number.isFinite(point.x) && Number.isFinite(point.y));

  ctx.save();
  ctx.strokeStyle = "#0f766e";
  ctx.lineWidth = 0.9;
  let previousPoint = null;
  let started = false;
  for (const point of series) {
    const px = toPixelX(point.x, plot, xMin, xMax);
    const py = toPixelY(point.y, plot, yMin, yMax);
    const discontinuous = previousPoint ? isPointGap(previousPoint, point) : false;
    if (!started || discontinuous) {
      if (started) {
        ctx.stroke();
        ctx.beginPath();
      } else {
        ctx.beginPath();
      }
      ctx.moveTo(px, py);
      started = true;
    } else {
      ctx.lineTo(px, py);
    }
    previousPoint = point;
  }
  if (started) {
    ctx.stroke();
  }
  ctx.restore();

  for (const point of series) {
    const px = toPixelX(point.x, plot, xMin, xMax);
    const py = toPixelY(point.y, plot, yMin, yMax);
    if (px < plot.left || px > plot.right || py < plot.top || py > plot.bottom) continue;
    const selected = point.index === state.selectedPointIndex;
    ctx.beginPath();
    ctx.fillStyle = selected ? "#c2410c" : "#0f766e";
    ctx.arc(px, py, selected ? 3.6 : 1.8, 0, Math.PI * 2);
    ctx.fill();
    state.pointCache.push({ x: px, y: py, index: point.index, radius: selected ? 10 : 7 });
  }

  if (state.drag?.mode === SELECT_MODE && state.drag.moved) {
    drawSelectionOutline(ctx, plot, state.drag.startPlotX, state.drag.currentPlotX);
  }

  ctx.fillStyle = "#15202b";
  ctx.font = '600 16px "Segoe UI"';
  ctx.fillText(`${getRunDisplayLabel(run)} speed profile`, plot.left, 18);
}

function drawGrid(ctx, plot, xMin, xMax, yMin, yMax) {
  ctx.save();
  ctx.strokeStyle = "rgba(93, 108, 122, 0.18)";
  ctx.lineWidth = 1;

  const xStep = niceStep((xMax - xMin) / 8);
  for (let value = Math.floor(xMin / xStep) * xStep; value <= xMax; value += xStep) {
    const px = toPixelX(value, plot, xMin, xMax);
    ctx.beginPath();
    ctx.moveTo(px, plot.top);
    ctx.lineTo(px, plot.bottom);
    ctx.stroke();
  }

  const yStep = niceStep((yMax - yMin) / 6);
  for (let value = Math.floor(yMin / yStep) * yStep; value <= yMax; value += yStep) {
    const py = toPixelY(value, plot, yMin, yMax);
    ctx.beginPath();
    ctx.moveTo(plot.left, py);
    ctx.lineTo(plot.right, py);
    ctx.stroke();
  }
  ctx.restore();
}

function drawAxisLabels(ctx, plot, xMin, xMax, yMin, yMax) {
  ctx.save();
  ctx.fillStyle = "#5d6c7a";
  ctx.font = '12px "Segoe UI"';

  ctx.textAlign = "center";
  const xStep = niceStep((xMax - xMin) / 8);
  for (let value = Math.floor(xMin / xStep) * xStep; value <= xMax; value += xStep) {
    const px = toPixelX(value, plot, xMin, xMax);
    ctx.fillText(value.toFixed(1), px, plot.bottom + 22);
  }

  ctx.textAlign = "right";
  const yStep = niceStep((yMax - yMin) / 6);
  for (let value = Math.floor(yMin / yStep) * yStep; value <= yMax; value += yStep) {
    const py = toPixelY(value, plot, yMin, yMax);
    ctx.fillText(value.toFixed(3), plot.left - 10, py + 4);
  }

  ctx.fillText("x", plot.right, plot.bottom + 42);
  ctx.save();
  ctx.translate(22, plot.top);
  ctx.rotate(-Math.PI / 2);
  ctx.fillText("speed (m/gt)", 0, 0);
  ctx.restore();
  ctx.restore();
}

function drawIdealLine(ctx, plot, xMin, xMax, yMin, yMax, run) {
  const targetSpeed = getTargetSpeedForRun(run);
  const py = toPixelY(targetSpeed, plot, yMin, yMax);
  ctx.save();
  ctx.strokeStyle = "rgba(194, 65, 12, 0.82)";
  ctx.lineWidth = 1;
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.moveTo(plot.left, py);
  ctx.lineTo(plot.right, py);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = "#c2410c";
  ctx.font = '12px "Segoe UI"';
  ctx.fillText(`ideal ${formatNumber(targetSpeed, 3)}`, plot.right - 72, py - 8);
  ctx.restore();
}

function drawIntervalBand(ctx, plot, xMin, xMax, intervalState, kind) {
  const interval = normalizeInterval(intervalState?.startX, intervalState?.endX);
  if (!interval) return;
  const start = Math.max(interval.startX, xMin);
  const end = Math.min(interval.endX, xMax);
  if (end <= start) return;

  const left = toPixelX(start, plot, xMin, xMax);
  const right = toPixelX(end, plot, xMin, xMax);
  ctx.save();
  if (kind === "preview") {
    ctx.fillStyle = "rgba(194, 65, 12, 0.07)";
    ctx.strokeStyle = "rgba(194, 65, 12, 0.42)";
    ctx.setLineDash([8, 6]);
  } else {
    ctx.fillStyle = "rgba(15, 118, 110, 0.035)";
    ctx.strokeStyle = "rgba(15, 118, 110, 0.18)";
  }
  ctx.lineWidth = 1;
  ctx.fillRect(left, plot.top, right - left, plot.bottom - plot.top);
  ctx.beginPath();
  ctx.moveTo(left, plot.top);
  ctx.lineTo(left, plot.bottom);
  ctx.moveTo(right, plot.top);
  ctx.lineTo(right, plot.bottom);
  ctx.stroke();

  if (kind !== "preview") {
    ctx.fillStyle = "rgba(15, 118, 110, 0.14)";
    ctx.fillRect(left, plot.top, Math.max(1, right - left), 2);
  }
  ctx.restore();
}

function drawSelectionOutline(ctx, plot, startPlotX, endPlotX) {
  const left = clamp(Math.min(startPlotX, endPlotX), plot.left, plot.right);
  const right = clamp(Math.max(startPlotX, endPlotX), plot.left, plot.right);
  if (right - left < 1) return;
  ctx.save();
  ctx.fillStyle = "rgba(194, 65, 12, 0.05)";
  ctx.strokeStyle = "rgba(194, 65, 12, 0.55)";
  ctx.lineWidth = 1;
  ctx.setLineDash([5, 5]);
  ctx.fillRect(left, plot.top, right - left, plot.bottom - plot.top);
  ctx.strokeRect(left, plot.top, right - left, plot.bottom - plot.top);
  ctx.restore();
}

function toPixelX(value, plot, xMin, xMax) {
  return plot.left + ((value - xMin) / (xMax - xMin || 1)) * (plot.right - plot.left);
}

function toPixelY(value, plot, yMin, yMax) {
  return plot.bottom - ((value - yMin) / (yMax - yMin || 1)) * (plot.bottom - plot.top);
}

function fromPixelX(pixel, plot, xMin, xMax) {
  return xMin + ((pixel - plot.left) / (plot.right - plot.left || 1)) * (xMax - xMin);
}

function fromPixelY(pixel, plot, yMin, yMax) {
  return yMin + ((plot.bottom - pixel) / (plot.bottom - plot.top || 1)) * (yMax - yMin);
}

function niceStep(rawStep) {
  const safeStep = rawStep > 0 ? rawStep : 1;
  const power = Math.pow(10, Math.floor(Math.log10(safeStep)));
  const ratio = safeStep / power;
  if (ratio <= 1) return 1 * power;
  if (ratio <= 2) return 2 * power;
  if (ratio <= 5) return 5 * power;
  return 10 * power;
}

function onWheel(event) {
  const run = getSelectedRun();
  if (!run || !state.view) return;
  event.preventDefault();

  const position = getCanvasPosition(event);
  const plot = getPlotRect();
  const scale = event.deltaY < 0 ? 0.88 : 1.14;
  const anchorX = fromPixelX(position.x, plot, state.view.xMin, state.view.xMax);
  const anchorY = fromPixelY(position.y, plot, state.view.yMin, state.view.yMax);

  state.view.xMin = anchorX - (anchorX - state.view.xMin) * scale;
  state.view.xMax = anchorX + (state.view.xMax - anchorX) * scale;
  state.view.yMin = anchorY - (anchorY - state.view.yMin) * scale;
  state.view.yMax = anchorY + (state.view.yMax - anchorY) * scale;
  enforceMinimumRanges();
  render();
}

function onPointerDown(event) {
  if (!state.view) return;
  const position = getCanvasPosition(event);
  const plot = getPlotRect();
  if (!isInsidePlot(position, plot)) return;

  state.drag = {
    pointerId: event.pointerId,
    mode: state.interactionMode,
    startClientX: event.clientX,
    startClientY: event.clientY,
    lastClientX: event.clientX,
    lastClientY: event.clientY,
    startPlotX: clamp(position.x, plot.left, plot.right),
    currentPlotX: clamp(position.x, plot.left, plot.right),
    startPlotY: clamp(position.y, plot.top, plot.bottom),
    currentPlotY: clamp(position.y, plot.top, plot.bottom),
    moved: false,
  };
  chartCanvas.dataset.dragging = "true";
  chartCanvas.setPointerCapture(event.pointerId);
}

function onPointerMove(event) {
  if (!state.drag || state.drag.pointerId !== event.pointerId || !state.view) return;
  const plot = getPlotRect();
  const position = getCanvasPosition(event);
  const dx = event.clientX - state.drag.startClientX;
  const dy = event.clientY - state.drag.startClientY;
  if (Math.abs(dx) > CLICK_DISTANCE_PX || Math.abs(dy) > CLICK_DISTANCE_PX) {
    state.drag.moved = true;
  }

  state.drag.currentPlotX = clamp(position.x, plot.left, plot.right);
  state.drag.currentPlotY = clamp(position.y, plot.top, plot.bottom);

  if (!state.drag.moved) return;

  if (state.drag.mode === PAN_MODE) {
    const xRange = state.view.xMax - state.view.xMin;
    const yRange = state.view.yMax - state.view.yMin;
    const xShift = ((event.clientX - state.drag.lastClientX) / (plot.right - plot.left)) * xRange;
    const yShift = ((event.clientY - state.drag.lastClientY) / (plot.bottom - plot.top)) * yRange;
    state.view.xMin -= xShift;
    state.view.xMax -= xShift;
    state.view.yMin += yShift;
    state.view.yMax += yShift;
    state.drag.lastClientX = event.clientX;
    state.drag.lastClientY = event.clientY;
    render();
    return;
  }

  const startX = fromPixelX(state.drag.startPlotX, plot, state.view.xMin, state.view.xMax);
  const currentX = fromPixelX(state.drag.currentPlotX, plot, state.view.xMin, state.view.xMax);
  state.previewInterval = normalizeInterval(startX, currentX);
  syncIntervalInputs(state.previewInterval);
  updateSummary();
  render();
}

function onPointerUp(event) {
  if (!state.drag || state.drag.pointerId !== event.pointerId) return;
  const drag = state.drag;
  cleanupDrag(event.pointerId);

  if (drag.moved && drag.mode === SELECT_MODE) {
    const interval =
      state.previewInterval ??
      normalizeInterval(
        fromPixelX(drag.startPlotX, getPlotRect(), state.view.xMin, state.view.xMax),
        fromPixelX(drag.currentPlotX, getPlotRect(), state.view.xMin, state.view.xMax),
      );
    if (interval) {
      state.interval.startX = interval.startX;
      state.interval.endX = interval.endX;
    }
    state.previewInterval = null;
    syncIntervalInputs();
    updateSummary();
    render();
    return;
  }

  state.previewInterval = null;
  handlePointSelection(event);
}

function onPointerCancel(event) {
  if (!state.drag || state.drag.pointerId !== event.pointerId) return;
  state.previewInterval = null;
  cleanupDrag(event.pointerId);
  syncIntervalInputs();
  updateSummary();
  render();
}

function onPointerLeave(event) {
  if (!state.drag || state.drag.pointerId !== event.pointerId) return;
  if (state.drag.mode !== SELECT_MODE) return;
  const plot = getPlotRect();
  const position = getCanvasPosition(event);
  state.drag.currentPlotX = clamp(position.x, plot.left, plot.right);
  state.drag.currentPlotY = clamp(position.y, plot.top, plot.bottom);
}

function cleanupDrag(pointerId) {
  state.drag = null;
  chartCanvas.dataset.dragging = "false";
  chartCanvas.releasePointerCapture?.(pointerId);
}

function handlePointSelection(event) {
  const position = getCanvasPosition(event);
  let nearest = null;
  for (const point of state.pointCache) {
    const distance = Math.hypot(point.x - position.x, point.y - position.y);
    if (distance <= point.radius && (!nearest || distance < nearest.distance)) {
      nearest = { ...point, distance };
    }
  }

  state.selectedPointIndex = nearest ? nearest.index : null;
  updateSummary();
  renderSelectedStructurePanels();
  render();
}

function normalizeInterval(startX, endX) {
  if (!Number.isFinite(startX) || !Number.isFinite(endX)) return null;
  return startX <= endX ? { startX, endX } : { startX: endX, endX: startX };
}

function parseIsoTime(value) {
  if (!value) return null;
  const ms = Date.parse(value);
  return Number.isFinite(ms) ? ms : null;
}

function parseTickIndex(point) {
  const rawTick = point?.tick_index;
  if (rawTick == null || rawTick === "") return null;
  const tick = Number(rawTick);
  return Number.isInteger(tick) ? tick : null;
}

function tickContinuity(previousPoint, nextPoint) {
  const previousTick = parseTickIndex(previousPoint);
  const nextTick = parseTickIndex(nextPoint);
  if (previousTick == null || nextTick == null) return null;
  return nextTick === previousTick + 1 ? "continuous" : "gap";
}

function isPointGap(previousPoint, nextPoint) {
  if (Math.abs(nextPoint.x - previousPoint.x) > MAX_CONTIGUOUS_DX) {
    return true;
  }
  // latest.log may flush delayed batches; continuous game ticks are stronger than wall-clock gaps.
  const tickState = tickContinuity(previousPoint, nextPoint);
  if (tickState === "continuous") {
    return false;
  }
  if (tickState === "gap") {
    return true;
  }
  const prevTime = parseIsoTime(previousPoint.log_time);
  const nextTime = parseIsoTime(nextPoint.log_time);
  if (prevTime != null && nextTime != null) {
    return (nextTime - prevTime) / 1000 > MAX_CONTIGUOUS_LOG_GAP_SECONDS;
  }
  return false;
}

function computeBlockDwellGroups(points) {
  const groups = [];
  let currentGroup = [];

  for (const point of points) {
    const blockX = Math.floor(Number.isFinite(point.x_raw) ? point.x_raw : point.x);
    if (!Number.isFinite(blockX)) continue;

    if (currentGroup.length === 0) {
      currentGroup = [{ ...point, blockX }];
      continue;
    }

    const previousPoint = currentGroup[currentGroup.length - 1];
    if (isPointGap(previousPoint, point) || blockX !== previousPoint.blockX) {
      groups.push(currentGroup);
      currentGroup = [{ ...point, blockX }];
    } else {
      currentGroup.push({ ...point, blockX });
    }
  }

  if (currentGroup.length > 0) groups.push(currentGroup);
  return groups;
}

function computeIntervalMetrics(run, startX, endX) {
  const interval = normalizeInterval(startX, endX);
  if (!interval) return null;

  const points = run.points
    .map((point) => ({ ...point, displayX: getPointX(point) }))
    .filter((point) => Number.isFinite(point.displayX) && point.displayX >= interval.startX && point.displayX <= interval.endX);
  if (points.length === 0) return null;

  const speedValues = points
    .map((point) => (point.speed != null ? point.speed : point.derived_speed))
    .filter((value) => Number.isFinite(value));
  const avgSpeed = speedValues.length ? speedValues.reduce((sum, value) => sum + value, 0) / speedValues.length : null;

  const dwellGroups = computeBlockDwellGroups(run.points).filter((group) => {
    const firstPoint = group[0];
    const lastPoint = group[group.length - 1];
    const firstX = getPointX(firstPoint);
    const lastX = getPointX(lastPoint);
    return Number.isFinite(firstX) && Number.isFinite(lastX) && firstX >= interval.startX && lastX <= interval.endX;
  });

  const targetDwellTicks = getTargetDwellTicksForRun(run);
  const tailGroup = dwellGroups.length ? dwellGroups[dwellGroups.length - 1] : null;
  const tailLastPoint = tailGroup?.[tailGroup.length - 1] ?? null;
  const tailLastX = tailLastPoint ? getPointX(tailLastPoint) : null;
  const hasTailTruncatedGroup =
    Boolean(tailGroup)
    && tailGroup.length < targetDwellTicks
    && Number.isFinite(tailLastX)
    && Math.abs(tailLastX - interval.endX) < 1e-6;
  const tailInferredHit = hasTailTruncatedGroup && isTailGroupInStablePattern(dwellGroups, targetDwellTicks);
  const scoredDwellGroups = hasTailTruncatedGroup && !tailInferredHit ? dwellGroups.slice(0, -1) : dwellGroups;
  const exactHitCount = scoredDwellGroups.filter((group) => group.length === targetDwellTicks).length;
  const hitCount = exactHitCount + (tailInferredHit ? 1 : 0);
  const firstPoint = points[0];
  const lastPoint = points[points.length - 1];
  const deltaX = lastPoint.displayX - firstPoint.displayX;
  const durationGt = lastPoint.tick_index - firstPoint.tick_index;
  const idealDistance = durationGt * getTargetSpeedForRun(run);
  const alignmentStartX = state.truncateEndAlignment ? Math.trunc(firstPoint.displayX) : firstPoint.displayX;
  const alignmentEndX = state.truncateEndAlignment ? Math.trunc(lastPoint.displayX) : lastPoint.displayX;
  const alignmentDeltaX = alignmentEndX - alignmentStartX;
  const endOffset = Number.isFinite(idealDistance) ? alignmentDeltaX - idealDistance : null;
  const endAlignmentRate =
    Number.isFinite(idealDistance) && idealDistance > 0 && Number.isFinite(endOffset)
      ? clampUnit(1 - Math.abs(endOffset) / idealDistance)
      : null;
  return {
    startX: interval.startX,
    endX: interval.endX,
    sampleCount: points.length,
    avgSpeed,
    twoGtHitRate: scoredDwellGroups.length ? hitCount / scoredDwellGroups.length : null,
    twoGtHitCount: hitCount,
    twoGtPairCount: scoredDwellGroups.length,
    twoGtBlockCount: scoredDwellGroups.length,
    tailTruncatedGroupCount: hasTailTruncatedGroup && !tailInferredHit ? 1 : 0,
    tailInferredHitCount: tailInferredHit ? 1 : 0,
    deltaX,
    durationGt,
    idealBlockCount: idealDistance,
    endOffset,
    endAlignmentRate,
  };
}

function isTailGroupInStablePattern(dwellGroups, targetDwellTicks, minPatternGroups = 8) {
  if (!dwellGroups.length) return false;
  const tailGroup = dwellGroups[dwellGroups.length - 1];
  if (!tailGroup || tailGroup.length >= targetDwellTicks) return false;
  const previousGroups = dwellGroups.slice(0, -1);
  if (previousGroups.length < minPatternGroups) return false;
  const recent = previousGroups.slice(-minPatternGroups);
  let previousBlock = null;
  for (const group of recent) {
    const blockX = group[0]?.blockX;
    if (group.length !== targetDwellTicks) return false;
    if (previousBlock != null && blockX !== previousBlock + 1) return false;
    previousBlock = blockX;
  }
  const tailBlock = tailGroup[0]?.blockX;
  return previousBlock != null && tailBlock === previousBlock + 1;
}

function applyIntervalFromInputs() {
  const startX = intervalStartInput.value === "" ? null : Number(intervalStartInput.value);
  const endX = intervalEndInput.value === "" ? null : Number(intervalEndInput.value);
  state.previewInterval = null;
  state.interval.startX = Number.isFinite(startX) ? startX : null;
  state.interval.endX = Number.isFinite(endX) ? endX : null;
  syncIntervalInputs();
  updateSummary();
  render();
}

function syncIntervalInputs(overrideInterval = null) {
  const interval = overrideInterval ?? normalizeInterval(state.interval.startX, state.interval.endX);
  intervalStartInput.value = interval ? Number(interval.startX).toFixed(3) : "";
  intervalEndInput.value = interval ? Number(interval.endX).toFixed(3) : "";
}

function setInteractionMode(mode) {
  state.interactionMode = mode;
  syncModeUi();
}

function setCoordinateMode(mode) {
  if (![ZERO_COORD_MODE, RAW_COORD_MODE].includes(mode) || state.coordinateMode === mode) return;
  const run = getSelectedRun();
  const previousMode = state.coordinateMode;

  const convertX = (value) => zeroXToDisplayX(run, displayXToZeroX(run, value, previousMode), mode);
  const convertInterval = (intervalState) => {
    const interval = normalizeInterval(intervalState?.startX, intervalState?.endX);
    if (!interval) return { startX: null, endX: null };
    return normalizeInterval(convertX(interval.startX), convertX(interval.endX)) ?? { startX: null, endX: null };
  };

  state.coordinateMode = mode;
  state.interval = convertInterval(state.interval);
  state.previewInterval = state.previewInterval ? convertInterval(state.previewInterval) : null;
  if (state.view) {
    const convertedView = normalizeInterval(convertX(state.view.xMin), convertX(state.view.xMax));
    if (convertedView) {
      state.view.xMin = convertedView.startX;
      state.view.xMax = convertedView.endX;
    } else {
      state.view = null;
    }
  }

  syncCoordinateModeUi();
  syncIntervalInputs();
  updateSummary();
  renderRunList();
  render();
}

function syncCoordinateModeUi() {
  const isZeroMode = state.coordinateMode === ZERO_COORD_MODE;
  zeroCoordButton.classList.toggle("active", isZeroMode);
  rawCoordButton.classList.toggle("active", !isZeroMode);
}

function setEndAlignmentMode(truncate) {
  state.truncateEndAlignment = Boolean(truncate);
  syncEndAlignmentModeUi();
  updateSummary();
}

function syncEndAlignmentModeUi() {
  preciseEndButton.classList.toggle("active", !state.truncateEndAlignment);
  truncatedEndButton.classList.toggle("active", state.truncateEndAlignment);
}

function syncModeUi() {
  const isSelect = state.interactionMode === SELECT_MODE;
  selectModeButton.classList.toggle("active", isSelect);
  panModeButton.classList.toggle("active", !isSelect);
  toolbarNote.textContent = isSelect
    ? "拖拽选区，滚轮缩放，点击折线点查看详情"
    : "拖拽平移，滚轮缩放，点击折线点查看详情";
  chartCanvas.dataset.mode = state.interactionMode;
}

function enforceMinimumRanges() {
  const minXRange = 0.2;
  const minYRange = 0.01;
  if (state.view.xMax - state.view.xMin < minXRange) {
    const center = (state.view.xMax + state.view.xMin) / 2;
    state.view.xMin = center - minXRange / 2;
    state.view.xMax = center + minXRange / 2;
  }
  if (state.view.yMax - state.view.yMin < minYRange) {
    const center = (state.view.yMax + state.view.yMin) / 2;
    state.view.yMin = center - minYRange / 2;
    state.view.yMax = center + minYRange / 2;
  }
}

function getCanvasPosition(event) {
  const rect = chartCanvas.getBoundingClientRect();
  return {
    x: ((event.clientX - rect.left) / (rect.width || 1)) * (state.canvas.width || rect.width || 1),
    y: ((event.clientY - rect.top) / (rect.height || 1)) * (state.canvas.height || rect.height || 1),
  };
}

function isInsidePlot(position, plot) {
  return (
    position.x >= plot.left &&
    position.x <= plot.right &&
    position.y >= plot.top &&
    position.y <= plot.bottom
  );
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function formatNumber(value, digits = 6) {
  if (!Number.isFinite(value)) return "-";
  return Number(value).toFixed(digits);
}

function formatSignedNumber(value, digits = 6) {
  if (!Number.isFinite(value)) return "-";
  const number = Number(value);
  return `${number > 0 ? "+" : ""}${number.toFixed(digits)}`;
}

function formatPercent(value) {
  if (!Number.isFinite(value)) return "-";
  return `${(Number(value) * 100).toFixed(2)}%`;
}

function setStatus(text) {
  document.getElementById("statusLine").textContent = text;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function safeFileStem(value) {
  return String(value || "waterway")
    .replace(/[\\/:*?"<>|]+/g, "_")
    .replace(/\s+/g, "_")
    .slice(0, 80) || "waterway";
}

loadRuns(false).catch((error) => {
  console.error(error);
  setStatus("加载失败");
});

window.__viewerDebug = {
  loadRuns: () => loadRuns(true),
  getSelectedRun,
  getEditorState: () => ({ ...editorState }),
  getEditorLaunchMode: () => editorLaunchMode(),
  getEditorStructure: () => structureFromEditor(),
  computeIntervalMetrics,
  resetView,
  setInteractionMode,
};
