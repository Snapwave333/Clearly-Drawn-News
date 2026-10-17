import * as THREE from 'three'
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { RGBELoader } from 'three/examples/jsm/loaders/RGBELoader.js'
import './style.css'

// ---------------------------------------------------------
// Global Setup & 4K High-Definition Renderer
// ---------------------------------------------------------
const canvas = document.querySelector('#office-canvas')
const scene = new THREE.Scene()
scene.background = new THREE.Color('#9fcde2')
// Fog density adjusted to be subtle so the 24-hour HDRI skybox and distant mountains are visible through picture windows
scene.fog = new THREE.FogExp2('#9fcde2', 0.002)

const loadedModels = {
  desk: null,
  laptop: null,
  chair: null,
  coffeeCart: null,
  projector: null,
  plant: null,
  coffeeTable: null
}

const loadedSkyboxes = {
  dawn: null,
  day: null,
  sunset: null,
  night: null
}

let pmremGenerator = null
let currentSkyPhase = ''

const renderer = new THREE.WebGLRenderer({
  canvas,
  antialias: true,
  powerPreference: 'high-performance'
})
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2))
renderer.shadowMap.enabled = true
renderer.shadowMap.type = THREE.PCFShadowMap
renderer.outputColorSpace = THREE.SRGBColorSpace
renderer.toneMapping = THREE.ACESFilmicToneMapping
renderer.toneMappingExposure = 1.15

const camera = new THREE.PerspectiveCamera(38, 1, 0.1, 400)
camera.position.set(55, 51, 60)

const controls = new OrbitControls(camera, canvas)
controls.enableDamping = true
controls.dampingFactor = 0.06
controls.maxPolarAngle = Math.PI * 0.46
controls.minDistance = 6
controls.maxDistance = 140
controls.target.set(0, 0, 0)

const palette = {
  plaster: 0xe8e2d6,
  plasterLight: 0xf5f1e8,
  ink: 0x17313b,
  sky: 0x8fc8e8,
  walnut: 0x6a4633,
  walnutDark: 0x4a2e22,
  plant: 0x4d7656,
  amber: 0xf2a65a,
  glass: 0xadd9e8,
  carpet: 0xbfc5bd,
  carpetRunner: 0x3d5059,
  chrome: 0xd9e1e8,
  stainless: 0x9ca8b0
}

const agents = []
const animatedProps = []
const doors = []
const obstacles = []
const messageSprites = []
const raycaster = new THREE.Raycaster()
const pointer = new THREE.Vector2()
const clock = new THREE.Clock()

let bootstrap = { agents: [], messages: [] }
let followMode = true
let followedIndex = 0
let lastScheduleMinute = -1
let lastSavedAt = 0
let daylight = 1
let manualMeetingOverride = false
let manualMeetingTimer = null

// ---------------------------------------------------------
// Wardrobe Catalog & 7-Day Outfits (Denver Synced)
// ---------------------------------------------------------
const wardrobeDays = [
  'Sunday Minimalist Softwear',
  'Monday Executive Tailoring',
  'Tuesday Smart Layered Studio',
  'Wednesday Midweek Statement Palette',
  'Thursday Deep-Work Knitwear',
  'Friday Studio Casual & Techwear',
  'Saturday Creator Lab Athleisure'
]

const styleProfiles = [
  { taste: 'Sartorial Minimalist', hairStyles: ['parted', 'slicked'], accessories: ['tortoiseshell glasses', 'gold watch'] },
  { taste: 'Cyberpunk Techwear', hairStyles: ['buzzcut', 'textured crop'], accessories: ['tech specs', 'lanyard'] },
  { taste: 'Studio Creative Art', hairStyles: ['afro', 'ponytail'], accessories: ['beret', 'studio badge'] },
  { taste: 'Vintage Academia', hairStyles: ['wavy bob', 'parted'], accessories: ['wireframe glasses', 'cardigan clip'] },
  { taste: 'Modern Media Director', hairStyles: ['slicked', 'textured crop'], accessories: ['studio monitors', 'chronograph'] },
  { taste: 'Clean Scandinavian', hairStyles: ['short crop', 'bun'], accessories: ['rimless glasses', 'minimalist ring'] }
]

const outfitPalettes = [
  [0x315a70, 0x243b48, 0x3a5948, 0x7b5268, 0x926342, 0x4b6577, 0x5b4d68],
  [0x8d4444, 0x486c83, 0x546342, 0x9a6b3d, 0x5f527c, 0x8b7456, 0x3e5e60],
  [0x48647d, 0xb06d59, 0x386653, 0x675281, 0x9b7553, 0x315b67, 0x6e4856],
  [0x2d4a58, 0x73513a, 0x456b5d, 0x824e4e, 0x3d4f61, 0x7a6344, 0x524357],
  [0x526b77, 0x8e5c46, 0x3f6154, 0x6e4a66, 0x80624a, 0x3f5c6b, 0x595163]
]

function prepareModel(modelScene) {
  modelScene.traverse(child => {
    if (child.isMesh) {
      child.castShadow = true
      child.receiveShadow = true
      if (child.material) {
        child.material.side = THREE.FrontSide
      }
    }
  })
  return modelScene
}

async function loadStudioAssets(onProgress) {
  const gltfLoader = new GLTFLoader()
  const rgbeLoader = new RGBELoader()

  onProgress?.('Loading 24-hour skyboxes & PBR lighting...')
  const skyboxDefs = [
    { key: 'dawn', path: './assets/skybox/dawn.hdr' },
    { key: 'day', path: './assets/skybox/day.hdr' },
    { key: 'sunset', path: './assets/skybox/sunset.hdr' },
    { key: 'night', path: './assets/skybox/night.hdr' }
  ]

  for (const item of skyboxDefs) {
    try {
      const texture = await rgbeLoader.loadAsync(item.path)
      texture.mapping = THREE.EquirectangularReflectionMapping
      let envMap = texture
      if (pmremGenerator) {
        envMap = pmremGenerator.fromEquirectangular(texture).texture
      }
      loadedSkyboxes[item.key] = { texture, envMap }
      console.log(`[Skybox] Loaded 24-hour HDRI: ${item.key}`)
    } catch (e) {
      console.warn(`[Skybox] Failed to load ${item.key}:`, e.message)
    }
  }

  onProgress?.('Loading photorealistic 3D props & furniture...')
  const modelDefs = [
    { key: 'desk', path: './assets/models/metal_office_desk/metal_office_desk_1k.gltf' },
    { key: 'laptop', path: './assets/models/classic_laptop/classic_laptop_1k.gltf' },
    { key: 'chair', path: './assets/models/modern_arm_chair_01/modern_arm_chair_01_1k.gltf' },
    { key: 'coffeeCart', path: './assets/models/CoffeeCart_01/CoffeeCart_01_1k.gltf' },
    { key: 'projector', path: './assets/models/projector_screen/projector_screen_1k.gltf' },
    { key: 'plant', path: './assets/models/potted_plant_01/potted_plant_01_1k.gltf' },
    { key: 'coffeeTable', path: './assets/models/modern_coffee_table_01/modern_coffee_table_01_1k.gltf' }
  ]

  for (const item of modelDefs) {
    try {
      const gltf = await gltfLoader.loadAsync(item.path)
      loadedModels[item.key] = prepareModel(gltf.scene)
      console.log(`[Asset] Loaded real 3D model: ${item.key}`)
    } catch (e) {
      console.warn(`[Asset] Failed to load model ${item.key}:`, e.message)
    }
  }
}

// ---------------------------------------------------------
// Materials & 3D Primitives Helpers
// ---------------------------------------------------------
function material(color, roughness = 0.78, metalness = 0.02) {
  return new THREE.MeshStandardMaterial({ color, roughness, metalness })
}

function meshBox(name, size, position, color, options = {}) {
  const geometry = new THREE.BoxGeometry(size[0], size[1], size[2])
  const body = new THREE.Mesh(geometry, options.material || material(color, options.roughness, options.metalness))
  body.name = name
  body.position.set(position[0], position[1], position[2])
  body.castShadow = options.castShadow ?? true
  body.receiveShadow = options.receiveShadow ?? true
  scene.add(body)
  return body
}

function addObstacle(x, z, width, depth) {
  obstacles.push({ minX: x - width / 2, maxX: x + width / 2, minZ: z - depth / 2, maxZ: z + depth / 2 })
}

// ---------------------------------------------------------
// Floor, Corridors & Wayfinding
// ---------------------------------------------------------
function makeFloor() {
  meshBox('foundation', [82, 0.6, 52], [0, -0.38, 0], 0xd2cbc0, { castShadow: false })
  const floor = meshBox('office floor', [78, 0.2, 48], [0, 0, 0], palette.plasterLight, { castShadow: false })
  floor.material.roughness = 0.95

  // Central corridor runner
  const corridor = meshBox('central carpet runner', [74, 0.04, 7.6], [0, 0.13, 0], palette.carpetRunner, { castShadow: false })
  corridor.material.roughness = 0.98

  // Wayfinding edge strips along corridor
  for (const z of [-3.8, 3.8]) {
    const strip = meshBox('corridor brass border', [74, 0.05, 0.08], [0, 0.14, z], palette.amber, { castShadow: false })
    strip.material.metalness = 0.6
  }

  // Floor seam lines for architectural rhythm
  for (let x = -36; x <= 36; x += 6) {
    const seam = meshBox('floor seam', [0.035, 0.02, 48], [x, 0.12, 0], 0xbeb8ae, { castShadow: false })
    seam.material.transparent = true
    seam.material.opacity = 0.28
  }
}

function wall(x, z, width, depth, height = 3.6, color = palette.plaster) {
  const body = meshBox('wall', [width, height, depth], [x, height / 2, z], color)
  addObstacle(x, z, width + 0.2, depth + 0.2)
  return body
}

function glassWall(x, z, width, depth, height = 3.3) {
  const glass = new THREE.MeshStandardMaterial({
    color: palette.glass,
    transparent: true,
    opacity: 0.24,
    roughness: 0.1,
    metalness: 0.1
  })
  const mesh = meshBox('glass wall', [width, height, depth], [x, height / 2, z], palette.glass, { material: glass })
  addObstacle(x, z, width + 0.1, depth + 0.1)
  return mesh
}

// ---------------------------------------------------------
// Office Furniture: Desk, Chair, Lamp, Props
// ---------------------------------------------------------
function desk(x, z, rotation = 0) {
  const group = new THREE.Group()

  // Real Poly Haven Metal Office Desk
  if (loadedModels.desk) {
    const realDesk = loadedModels.desk.clone(true)
    realDesk.scale.set(1.15, 1.0, 1.0)
    realDesk.position.set(0, 0, 0)
    group.add(realDesk)
  } else {
    // Desktop fallback
    const top = new THREE.Mesh(new THREE.BoxGeometry(2.35, 0.12, 0.95), material(palette.walnut, 0.62))
    top.position.y = 0.76
    top.castShadow = true
    group.add(top)

    // Metal legs
    for (const dx of [-0.95, 0.95]) {
      const leg = new THREE.Mesh(new THREE.BoxGeometry(0.08, 0.74, 0.75), material(0x38464a, 0.5, 0.4))
      leg.position.set(dx, 0.37, 0)
      group.add(leg)
    }
  }

  // Real Poly Haven Classic Laptop
  if (loadedModels.laptop) {
    const laptop = loadedModels.laptop.clone(true)
    laptop.scale.set(0.72, 0.72, 0.72)
    laptop.position.set(0, 0.79, 0.05)
    laptop.rotation.y = Math.PI
    group.add(laptop)
  }

  // Dual high-definition workstation monitors
  for (const side of [-0.55, 0.55]) {
    const monitorFrame = new THREE.Mesh(new THREE.BoxGeometry(0.78, 0.48, 0.06), material(0x182429, 0.3, 0.2))
    monitorFrame.position.set(side, 1.15, -0.15)
    monitorFrame.rotation.y = Math.PI + (side * -0.15)
    group.add(monitorFrame)

    const screen = new THREE.Mesh(new THREE.PlaneGeometry(0.72, 0.42), new THREE.MeshBasicMaterial({ color: side < 0 ? 0x5cb2c9 : 0x4896a6 }))
    screen.position.set(side, 1.15, -0.182)
    screen.rotation.y = Math.PI + (side * -0.15)
    group.add(screen)
    animatedProps.push({ type: 'screen', object: screen, phase: Math.random() * 10 })
  }

  // Desk task lamp
  const lampBase = new THREE.Mesh(new THREE.CylinderGeometry(0.12, 0.14, 0.04, 12), material(palette.amber, 0.4, 0.6))
  lampBase.position.set(0.9, 0.84, 0.15)
  group.add(lampBase)
  const lampPole = new THREE.Mesh(new THREE.CylinderGeometry(0.02, 0.02, 0.42, 8), material(0x222222, 0.3, 0.7))
  lampPole.position.set(0.9, 1.05, 0.15)
  group.add(lampPole)
  const lampHead = new THREE.Mesh(new THREE.ConeGeometry(0.15, 0.18, 12), material(palette.amber, 0.4, 0.6))
  lampHead.position.set(0.85, 1.25, 0.15)
  lampHead.rotation.z = Math.PI / 4
  group.add(lampHead)
  const lampLight = new THREE.PointLight(0xfff0d0, 0.4, 3.2, 2)
  lampLight.position.set(0.85, 1.2, 0.15)
  group.add(lampLight)
  animatedProps.push({ type: 'taskLight', object: lampLight, phase: x + z })

  // Coffee mug on desk with delicate steam
  const mug = new THREE.Mesh(new THREE.CylinderGeometry(0.06, 0.06, 0.11, 10), material(0xf0ece1, 0.4))
  mug.position.set(-0.88, 0.87, 0.2)
  group.add(mug)
  for (let s = 0; s < 3; s++) {
    const puff = new THREE.Mesh(new THREE.SphereGeometry(0.025, 6, 5), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.25 }))
    puff.position.set(-0.88, 0.95 + s * 0.08, 0.2)
    group.add(puff)
    animatedProps.push({ type: 'steam', object: puff, phase: s * 1.5 + x })
  }

  group.position.set(x, 0.08, z)
  group.rotation.y = rotation
  scene.add(group)
  addObstacle(x, z, rotation ? 1.2 : 2.5, rotation ? 2.5 : 1.2)
  return group
}

function chair(x, z, rotation = 0, color = 0x40616b) {
  const group = new THREE.Group()

  // Real Poly Haven Modern Armchair
  if (loadedModels.chair) {
    const realChair = loadedModels.chair.clone(true)
    realChair.scale.set(0.85, 0.85, 0.85)
    realChair.position.set(0, 0, 0)
    realChair.rotation.y = Math.PI
    group.add(realChair)
  } else {
    // Fallback chair
    const seat = new THREE.Mesh(new THREE.BoxGeometry(0.58, 0.11, 0.58), material(color, 0.8))
    seat.position.y = 0.5
    group.add(seat)
    const back = new THREE.Mesh(new THREE.BoxGeometry(0.56, 0.72, 0.11), material(color, 0.8))
    back.position.set(0, 0.8, 0.25)
    group.add(back)
    const stem = new THREE.Mesh(new THREE.CylinderGeometry(0.045, 0.045, 0.45, 8), material(0x344247, 0.5, 0.4))
    stem.position.y = 0.25
    group.add(stem)
  }

  group.position.set(x, 0.08, z)
  group.rotation.y = rotation
  scene.add(group)
  return group
}

function plant(x, z, scale = 1) {
  const group = new THREE.Group()

  // Real Poly Haven Potted Plant
  if (loadedModels.plant) {
    const realPlant = loadedModels.plant.clone(true)
    realPlant.scale.set(0.85 * scale, 0.85 * scale, 0.85 * scale)
    realPlant.position.set(0, 0, 0)
    group.add(realPlant)
  } else {
    // Fallback plant
    const pot = new THREE.Mesh(new THREE.CylinderGeometry(0.28 * scale, 0.2 * scale, 0.48 * scale, 12), material(0xb87955))
    pot.position.y = 0.24 * scale
    group.add(pot)
    for (let i = 0; i < 8; i++) {
      const leaf = new THREE.Mesh(new THREE.SphereGeometry(0.25 * scale, 8, 6), material(i % 2 ? 0x5b825b : 0x416d4e))
      leaf.scale.set(0.55, 1.5, 0.3)
      leaf.position.set(Math.sin(i * 2.3) * 0.26 * scale, (0.65 + (i % 3) * 0.18) * scale, Math.cos(i * 2.3) * 0.26 * scale)
      leaf.rotation.z = Math.sin(i) * 0.48
      group.add(leaf)
    }
  }

  group.position.set(x, 0.08, z)
  scene.add(group)
  animatedProps.push({ type: 'plant', object: group, phase: Math.random() * 10 })
}

// ---------------------------------------------------------
// 38 Dedicated Offices with Working Doors & Windows
// ---------------------------------------------------------
function createOffice(index, count, definition) {
  const north = index < Math.ceil(count / 2)
  const rowIndex = north ? index : index - Math.ceil(count / 2)
  const rowCount = Math.ceil(count / 2)
  const totalWidth = 74
  const width = totalWidth / rowCount
  const x = -37 + width * rowIndex + width / 2
  const z = north ? 18.8 : -18.8
  const frontZ = north ? 16.4 : -16.4
  const backZ = north ? 22.2 : -22.2

  // Dividing side walls between offices
  if (rowIndex === 0) wall(-37, z, 0.12, 6.2)
  wall(x + width / 2, z, 0.12, 6.2)

  // Exterior picture window looking out onto Denver skyline and Rocky Mountains
  const windowGlass = glassWall(x, backZ, width - 0.12, 0.08, 3.2)
  windowGlass.material.opacity = 0.35

  // Window mullions for realism
  const mullion = meshBox('window mullion', [0.08, 3.2, 0.1], [x, 1.6, backZ], 0x243338)

  // Front partition wall with doorway opening
  const doorWidth = 1.35
  const wallSegment = Math.max(0.2, (width - doorWidth) / 2)
  wall(x - width / 2 + wallSegment / 2, frontZ, wallSegment, 0.12, 1.8)
  wall(x + width / 2 - wallSegment / 2, frontZ, wallSegment, 0.12, 1.8)

  // Automated working frosted glass door
  const doorPivot = new THREE.Group()
  doorPivot.position.set(x - doorWidth / 2, 0.05, frontZ)
  const doorMaterial = new THREE.MeshStandardMaterial({
    color: 0xcde8ee,
    transparent: true,
    opacity: 0.55,
    roughness: 0.18,
    metalness: 0.08
  })
  const doorPane = new THREE.Mesh(new THREE.BoxGeometry(doorWidth, 2.35, 0.06), doorMaterial)
  doorPane.position.set(doorWidth / 2, 1.18, 0)
  doorPivot.add(doorPane)

  // Door handle (brushed stainless steel)
  const handle = new THREE.Mesh(new THREE.BoxGeometry(0.04, 0.32, 0.12), material(palette.stainless, 0.2, 0.9))
  handle.position.set(doorWidth - 0.12, 1.18, 0.04)
  doorPivot.add(handle)
  scene.add(doorPivot)

  doors.push({
    object: doorPivot,
    x,
    z: frontZ,
    sign: north ? -1 : 1,
    isOpen: false,
    doorWidth
  })

  // Office interior: Desk & Chair
  desk(x, north ? 19.6 : -19.6, north ? 0 : Math.PI)
  chair(x, north ? 18.5 : -18.5, north ? 0 : Math.PI, 0x3d5460 + (index % 4) * 0x070402)

  // Personalized plant every few offices
  if (index % 2 === 0) plant(x + width * 0.3, north ? 21.1 : -21.1, 0.62)

  // Office label plaque above door
  const plaque = meshBox('office plaque', [0.8, 0.2, 0.04], [x, 2.65, frontZ + (north ? -0.08 : 0.08)], palette.ink)

  return {
    center: new THREE.Vector3(x, 0.08, z),
    work: new THREE.Vector3(x, 0.08, north ? 18.3 : -18.3),
    door: new THREE.Vector3(x, 0.08, north ? 15.6 : -15.6),
    label: `${north ? 'North' : 'South'} ${String(rowIndex + 1).padStart(2, '0')}`,
    doorRef: doorPivot
  }
}

// ---------------------------------------------------------
// Grand Executive Boardroom (All 38 Agents Have a Seat)
// ---------------------------------------------------------
function buildMeetingRoom(agentCount) {
  const x = 14
  const z = 5.8
  const width = 28
  const depth = 15

  // Glass boardroom perimeter walls with privacy frosting
  glassWall(x, z - depth / 2, width, 0.1, 3.4)
  glassWall(x, z + depth / 2, width, 0.1, 3.4)
  glassWall(x - width / 2, z - 4.2, 0.1, 6.6, 3.4)
  glassWall(x - width / 2, z + 4.2, 0.1, 6.6, 3.4)
  glassWall(x + width / 2, z, 0.1, depth, 3.4)

  // Double glass boardroom entrance doors
  const boardroomDoorL = new THREE.Group()
  boardroomDoorL.position.set(x - width / 2, 0.05, z - 1.2)
  const doorPanelL = new THREE.Mesh(new THREE.BoxGeometry(0.06, 2.4, 1.2), material(0xc2e2e9, 0.2, 0.1))
  doorPanelL.position.set(0, 1.2, 0.6)
  boardroomDoorL.add(doorPanelL)
  scene.add(boardroomDoorL)

  const boardroomDoorR = new THREE.Group()
  boardroomDoorR.position.set(x - width / 2, 0.05, z + 1.2)
  const doorPanelR = new THREE.Mesh(new THREE.BoxGeometry(0.06, 2.4, 1.2), material(0xc2e2e9, 0.2, 0.1))
  doorPanelR.position.set(0, 1.2, -0.6)
  boardroomDoorR.add(doorPanelR)
  scene.add(boardroomDoorR)

  doors.push({ object: boardroomDoorL, x: x - width / 2, z, sign: -1, isOpen: false })
  doors.push({ object: boardroomDoorR, x: x - width / 2, z, sign: 1, isOpen: false })

  // Grand conference table (walnut with brass edge trim)
  const table = meshBox('boardroom table', [13.2, 0.78, 4.4], [x, 0.39, z], palette.walnut)
  meshBox('table brass inlay', [13.4, 0.05, 4.6], [x, 0.79, z], palette.amber, { roughness: 0.3, metalness: 0.6 })
  addObstacle(x, z, 14, 5)

  // Center conference speakerphone & water carafes
  const speakerphone = new THREE.Mesh(new THREE.CylinderGeometry(0.24, 0.28, 0.08, 12), material(0x222222, 0.4, 0.6))
  speakerphone.position.set(x, 0.85, z)
  scene.add(speakerphone)

  // 38 GUARANTEED INDIVIDUAL SEATS FOR EVERY AGENT:
  // Ring 1: 22 Executive chairs surrounding the central conference table
  // Ring 2: 16 Executive gallery chairs along the boardroom perimeter with personal desks
  const seats = []

  // Top and bottom sides of main table (8 seats each = 16)
  for (let c = 0; c < 8; c++) {
    const seatX = x - 5.2 + c * 1.5
    // North side of table
    const zN = z + 2.7
    chair(seatX, zN, Math.PI, 0x6e527d)
    seats.push(new THREE.Vector3(seatX, 0.08, zN - 0.4))

    // South side of table
    const zS = z - 2.7
    chair(seatX, zS, 0, 0x6e527d)
    seats.push(new THREE.Vector3(seatX, 0.08, zS + 0.4))
  }

  // East and West ends of main table (3 seats each = 6, total 22 table seats)
  for (let r = 0; r < 3; r++) {
    const seatZ = z - 1.2 + r * 1.2
    // West end
    const xW = x - 7.2
    chair(xW, seatZ, Math.PI / 2, 0x6e527d)
    seats.push(new THREE.Vector3(xW + 0.4, 0.08, seatZ))

    // East end
    const xE = x + 7.2
    chair(xE, seatZ, -Math.PI / 2, 0x6e527d)
    seats.push(new THREE.Vector3(xE - 0.4, 0.08, seatZ))
  }

  // Ring 2: 16 Executive Gallery Tier Seats (8 North wall, 8 South wall)
  // Ensures all 38 agents have an expansive, comfortable physical seat!
  for (let g = 0; g < 8; g++) {
    const gx = x - 5.2 + g * 1.5
    // North gallery tier
    chair(gx, z + 5.6, Math.PI, 0x5a4667)
    seats.push(new THREE.Vector3(gx, 0.08, z + 5.0))

    // South gallery tier
    chair(gx, z - 5.6, 0, 0x5a4667)
    seats.push(new THREE.Vector3(gx, 0.08, z - 5.0))
  }

  // Real Poly Haven Projector Presentation Screen (East wall)
  if (loadedModels.projector) {
    const screenModel = loadedModels.projector.clone(true)
    screenModel.scale.set(2.5, 2.3, 2.5)
    screenModel.position.set(x + width / 2 - 0.55, 0.08, z)
    screenModel.rotation.y = -Math.PI / 2
    scene.add(screenModel)

    const screen = new THREE.Mesh(new THREE.PlaneGeometry(3.6, 2.1), new THREE.MeshBasicMaterial({ color: 0x4f8899 }))
    screen.position.set(x + width / 2 - 0.58, 2.1, z)
    screen.rotation.y = -Math.PI / 2
    scene.add(screen)
    animatedProps.push({ type: 'presentation', object: screen, phase: 0 })
  } else {
    // 4K Ultra-Wide Boardroom Presentation Display fallback (East wall)
    const display = meshBox('meeting display', [0.18, 2.8, 6.2], [x + width / 2 - 0.16, 2.0, z], 0x15252b)
    const screen = new THREE.Mesh(new THREE.PlaneGeometry(5.8, 2.5), new THREE.MeshBasicMaterial({ color: 0x4f8899 }))
    screen.position.set(x + width / 2 - 0.26, 2.0, z)
    screen.rotation.y = -Math.PI / 2
    scene.add(screen)
    animatedProps.push({ type: 'presentation', object: screen, phase: 0 })
  }

  return seats
}

// ---------------------------------------------------------
// Break Room / Kitchenette (4K HD Beautifully Detailed)
// ---------------------------------------------------------
function buildBreakRoom() {
  const x = -26
  const z = 5.8
  const width = 18
  const depth = 15

  // Glass partition enclosure
  glassWall(x - width / 2, z, 0.1, depth)
  glassWall(x, z - depth / 2, width, 0.1)
  glassWall(x, z + depth / 2, width, 0.1)
  glassWall(x + width / 2, z - 4.5, 0.1, 6)
  glassWall(x + width / 2, z + 4.5, 0.1, 6)

  // 1. Luxury Quartz Countertop & Dark Wood Cabinetry
  const counterZ = z + depth / 2 - 1.2
  const counter = meshBox('kitchen counter base', [11.5, 0.94, 1.4], [x, 0.47, counterZ], 0x2e3d40)
  meshBox('quartz countertop', [11.8, 0.1, 1.55], [x, 0.99, counterZ], 0xedeae4, { roughness: 0.35 })
  addObstacle(x, counterZ, 12, 1.6)

  // Stainless Steel Undermount Sink & Gooseneck Faucet
  meshBox('sink basin', [1.4, 0.28, 0.9], [x + 2.2, 0.85, counterZ], 0x8fa0a8, { metalness: 0.8, roughness: 0.2 })
  const faucet = new THREE.Mesh(new THREE.TorusGeometry(0.18, 0.025, 8, 16, Math.PI), material(palette.chrome, 0.2, 0.9))
  faucet.position.set(x + 2.2, 1.22, counterZ + 0.3)
  faucet.rotation.z = Math.PI / 2
  scene.add(faucet)

  // 2. Real Commercial Artisan Coffee Cart Station
  const coffeeX = x - 3.8
  if (loadedModels.coffeeCart) {
    const realCart = loadedModels.coffeeCart.clone(true)
    realCart.scale.set(1.0, 1.0, 1.0)
    realCart.position.set(coffeeX, 0.08, counterZ - 0.45)
    realCart.rotation.y = Math.PI
    scene.add(realCart)
  } else {
    // Fallback espresso station
    const coffeeStation = meshBox('espresso machine', [1.4, 0.95, 0.8], [coffeeX, 1.52, counterZ], 0x222b2e, { metalness: 0.7, roughness: 0.3 })
    const steamWand = new THREE.Mesh(new THREE.CylinderGeometry(0.015, 0.015, 0.35, 8), material(palette.chrome, 0.1, 0.95))
    steamWand.position.set(coffeeX - 0.45, 1.35, counterZ - 0.4)
    steamWand.rotation.z = 0.3
    scene.add(steamWand)
    const carafe = new THREE.Mesh(new THREE.CylinderGeometry(0.2, 0.22, 0.42, 12), new THREE.MeshStandardMaterial({ color: 0x2d1a0e, transparent: true, opacity: 0.8, roughness: 0.2 }))
    carafe.position.set(coffeeX + 0.42, 1.25, counterZ - 0.25)
    scene.add(carafe)
  }

  // Delicate animated rising steam particles from espresso & carafe
  for (let i = 0; i < 9; i++) {
    const puff = new THREE.Mesh(new THREE.SphereGeometry(0.06 + (i % 3) * 0.02, 7, 5), new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.28 }))
    puff.position.set(coffeeX + Math.sin(i) * 0.18, 1.8 + i * 0.18, counterZ - 0.3)
    scene.add(puff)
    animatedProps.push({ type: 'steam', object: puff, phase: i * 0.8 })
  }

  // 3. Water Cooler & Bottle Dispenser Station
  const waterX = x - 0.8
  meshBox('water cooler stand', [0.85, 1.05, 0.85], [waterX, 1.52, counterZ], 0xdedcd7)
  const waterJug = new THREE.Mesh(
    new THREE.CylinderGeometry(0.36, 0.38, 1.1, 16),
    new THREE.MeshStandardMaterial({ color: 0x6ec0db, transparent: true, opacity: 0.55, roughness: 0.15 })
  )
  waterJug.position.set(waterX, 2.55, counterZ)
  scene.add(waterJug)
  animatedProps.push({ type: 'water', object: waterJug, phase: 0 })

  // Hot (red) and Cold (cyan) spigot handles
  const tapCold = new THREE.Mesh(new THREE.BoxGeometry(0.04, 0.1, 0.08), material(0x2a9bc4))
  tapCold.position.set(waterX - 0.12, 1.85, counterZ - 0.45)
  scene.add(tapCold)
  const tapHot = new THREE.Mesh(new THREE.BoxGeometry(0.04, 0.1, 0.08), material(0xd94436))
  tapHot.position.set(waterX + 0.12, 1.85, counterZ - 0.45)
  scene.add(tapHot)

  // Paper cup dispenser cone
  const cupDispenser = new THREE.Mesh(new THREE.CylinderGeometry(0.08, 0.06, 0.8, 10), material(0x384c54, 0.4))
  cupDispenser.position.set(waterX + 0.48, 2.2, counterZ - 0.2)
  scene.add(cupDispenser)

  // 4. Glass-Front Glowing Vending Machine (4 Shelves of Snacks & Drinks)
  const vendingX = x + 4.2
  const vending = meshBox('vending machine', [1.8, 2.85, 1.25], [vendingX, 1.5, counterZ], 0x1f2e34)

  // Illuminated top header marquee ("ATLAS REFRESH")
  const marquee = meshBox('vending marquee', [1.65, 0.35, 0.1], [vendingX, 2.72, counterZ - 0.65], palette.amber)
  marquee.material.emissive = new THREE.Color(0xf2a65a)
  marquee.material.emissiveIntensity = 0.8

  // Transparent glass front
  const vendingGlass = new THREE.Mesh(
    new THREE.PlaneGeometry(1.55, 1.7),
    new THREE.MeshStandardMaterial({ color: 0x9be0f0, transparent: true, opacity: 0.28, roughness: 0.1 })
  )
  vendingGlass.position.set(vendingX, 1.55, counterZ - 0.64)
  scene.add(vendingGlass)

  // Internal glowing light casting cyan radiance onto the floor
  const vendingGlow = new THREE.PointLight(0x56c4e8, 1.5, 6, 2)
  vendingGlow.position.set(0, 0.2, -0.9)
  vending.add(vendingGlow)

  // Shelves with colored snack items & beverage cans inside
  for (let s = 0; s < 4; s++) {
    const shelfY = 0.95 + s * 0.42
    const shelf = meshBox('vending shelf', [1.5, 0.03, 0.8], [vendingX, shelfY, counterZ - 0.2], 0x48585f)
    for (let item = 0; item < 5; item++) {
      const itemMesh = new THREE.Mesh(
        new THREE.BoxGeometry(0.18, 0.22, 0.12),
        material([0xcc4433, 0x3388bb, 0x44aa55, 0xeebb33, 0x9955bb][item % 5])
      )
      itemMesh.position.set(vendingX - 0.6 + item * 0.3, shelfY + 0.12, counterZ - 0.2)
      scene.add(itemMesh)
    }
  }

  // 5. Modern Coffee Tables & Lounge Gathering Area
  if (loadedModels.coffeeTable) {
    const table1 = loadedModels.coffeeTable.clone(true)
    table1.scale.set(1.6, 1.4, 1.6)
    table1.position.set(x - 2.8, 0.08, z - 2.2)
    scene.add(table1)
    addObstacle(x - 2.8, z - 2.2, 2.6, 2.6)

    const table2 = loadedModels.coffeeTable.clone(true)
    table2.scale.set(1.6, 1.4, 1.6)
    table2.position.set(x + 2.8, 0.08, z - 2.2)
    scene.add(table2)
    addObstacle(x + 2.8, z - 2.2, 2.6, 2.6)
  } else {
    // Fallback cafe tables
    const cafeTable1 = new THREE.Mesh(new THREE.CylinderGeometry(1.6, 1.6, 0.1, 20), material(palette.walnut, 0.5))
    cafeTable1.position.set(x - 2.8, 0.88, z - 2.2)
    cafeTable1.castShadow = true
    scene.add(cafeTable1)
    addObstacle(x - 2.8, z - 2.2, 3.4, 3.4)

    const cafeTable2 = new THREE.Mesh(new THREE.CylinderGeometry(1.6, 1.6, 0.1, 20), material(palette.walnut, 0.5))
    cafeTable2.position.set(x + 2.8, 0.88, z - 2.2)
    cafeTable2.castShadow = true
    scene.add(cafeTable2)
    addObstacle(x + 2.8, z - 2.2, 3.4, 3.4)
  }

  // Stools around cafe tables
  for (let i = 0; i < 4; i++) {
    const angle = (i / 4) * Math.PI * 2
    chair(x - 2.8 + Math.cos(angle) * 2.2, z - 2.2 + Math.sin(angle) * 2.2, angle + Math.PI, 0x486470)
    chair(x + 2.8 + Math.cos(angle) * 2.2, z - 2.2 + Math.sin(angle) * 2.2, angle + Math.PI, 0x486470)
  }

  // Break spots where idle agents gather to sip coffee, drink water, get snacks, or sit
  const spots = [
    new THREE.Vector3(coffeeX, 0.08, counterZ - 1.2), // At espresso machine
    new THREE.Vector3(waterX, 0.08, counterZ - 1.2),   // At water cooler
    new THREE.Vector3(vendingX, 0.08, counterZ - 1.2), // At vending machine
    new THREE.Vector3(x - 2.8, 0.08, z - 0.8),         // At cafe table 1
    new THREE.Vector3(x - 2.8, 0.08, z - 3.6),
    new THREE.Vector3(x + 2.8, 0.08, z - 0.8),         // At cafe table 2
    new THREE.Vector3(x + 2.8, 0.08, z - 3.6),
    new THREE.Vector3(x - 5.5, 0.08, z + 1.2),         // Lounge counter edge
    new THREE.Vector3(x + 5.5, 0.08, z + 1.2)
  ]

  plant(x - width / 2 + 1.5, z - depth / 2 + 1.5, 1.2)
  plant(x + width / 2 - 1.5, z - depth / 2 + 1.5, 1.2)

  return spots
}

// ---------------------------------------------------------
// Exterior Denver Skyline & Rocky Mountain Vista
// ---------------------------------------------------------
function buildSkyline() {
  const skyline = new THREE.Group()
  for (let i = 0; i < 38; i++) {
    const width = 2.4 + (i * 7 % 5)
    const height = 7 + (i * 13 % 22)
    const building = new THREE.Mesh(
      new THREE.BoxGeometry(width, height, 3.4),
      material(i % 3 === 0 ? 0x4f6b75 : 0x677e87, 0.85)
    )
    building.position.set(-84 + i * 4.6, height / 2 - 1, -38 - (i % 4) * 3)
    skyline.add(building)

    // Window light strips on skyscrapers
    for (let y = 2.2; y < height - 1.5; y += 2.2) {
      const light = new THREE.Mesh(
        new THREE.PlaneGeometry(width * 0.72, 0.5),
        new THREE.MeshBasicMaterial({ color: i % 2 === 0 ? 0xffdf99 : 0x99e5ff, transparent: true, opacity: 0.15 })
      )
      light.position.set(0, y - height / 2 + 1, 1.72)
      building.add(light)
      animatedProps.push({ type: 'window', object: light, phase: i * 1.5 + y })
    }
  }
  scene.add(skyline)

  // Majestic Rocky Mountains in the distance
  const mountainMat = new THREE.MeshStandardMaterial({ color: 0x72898c, roughness: 0.98, flatShading: true })
  for (let i = 0; i < 19; i++) {
    const mountain = new THREE.Mesh(new THREE.ConeGeometry(9 + (i % 6), 13 + (i % 5), 4), mountainMat)
    mountain.position.set(-88 + i * 10, 2.5, -68 - (i % 3) * 4)
    mountain.rotation.y = Math.PI / 4
    scene.add(mountain)
  }
}

// ---------------------------------------------------------
// Lighting System & Ceiling Troffers
// ---------------------------------------------------------
function buildLighting() {
  if (!pmremGenerator) {
    pmremGenerator = new THREE.PMREMGenerator(renderer)
    pmremGenerator.compileEquirectangularShader()
  }

  const hemisphere = new THREE.HemisphereLight(0xbde6f4, 0xb8aa94, 1.45)
  scene.add(hemisphere)

  const sun = new THREE.DirectionalLight(0xffe2b3, 3.2)
  sun.position.set(-28, 48, -25)
  sun.castShadow = true
  sun.shadow.mapSize.set(2048, 2048)
  sun.shadow.camera.left = -55
  sun.shadow.camera.right = 55
  sun.shadow.camera.top = 45
  sun.shadow.camera.bottom = -45
  sun.shadow.camera.near = 0.5
  sun.shadow.camera.far = 150
  sun.shadow.bias = -0.0004
  scene.add(sun)

  scene.userData.hemisphere = hemisphere
  scene.userData.sun = sun

  // Office ceiling light troffers
  for (let x = -34; x <= 34; x += 7.5) {
    for (const z of [-14, 0, 14]) {
      const fixture = meshBox('ceiling troffer', [2.8, 0.06, 0.6], [x, 3.8, z], 0xf6eee0, { castShadow: false })
      const light = new THREE.PointLight(0xffe7bf, 0, 10, 2)
      light.position.set(x, 3.45, z)
      scene.add(light)
      animatedProps.push({ type: 'ceilingLight', object: light, fixture, phase: x + z })
    }
  }

  // Ceiling fans
  for (const x of [-14, 6, 26]) {
    const fan = new THREE.Group()
    fan.position.set(x, 3.5, 0)
    const hub = new THREE.Mesh(new THREE.CylinderGeometry(0.14, 0.18, 0.28, 10), material(0x31444a, 0.5, 0.5))
    fan.add(hub)
    for (let i = 0; i < 4; i++) {
      const blade = new THREE.Mesh(new THREE.BoxGeometry(2.8, 0.04, 0.22), material(0x6e8085, 0.6, 0.2))
      blade.position.x = 1.2
      const pivot = new THREE.Group()
      pivot.rotation.y = (i * Math.PI) / 2
      pivot.add(blade)
      fan.add(pivot)
    }
    scene.add(fan)
    animatedProps.push({ type: 'fan', object: fan, phase: x })
  }
}

// ---------------------------------------------------------
// Agent Instantiation with Distinct Taste, Wardrobe & Style
// ---------------------------------------------------------
function titleCase(value) {
  return (value || '').split('_').map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ')
}

function hashValue(value) {
  let hash = 2166136261
  for (let i = 0; i < value.length; i++) hash = Math.imul(hash ^ value.charCodeAt(i), 16777619)
  return Math.abs(hash)
}

function makeAgent(definition, index, office) {
  const seed = hashValue(definition.role)
  const group = new THREE.Group()
  group.userData.agentIndex = index

  // MetaHuman Skin Tone Palette (diverse realistic Fitzpatrick skin phototypes)
  const skinColors = [0xf5d0b5, 0xdfab82, 0xc68b59, 0x8d5524, 0x58341e, 0x3b2219, 0xfce3d2, 0xde9b72]
  const hairColors = [0x1a110d, 0x3d2314, 0x6a4023, 0xb58b57, 0x0c0c0c, 0x8a3324, 0x4a4f54, 0x2b1d14]
  const eyeColors = [0x3c2817, 0x4d6c54, 0x2e536f, 0x634d3b, 0x23445d, 0x41352a]
  
  const skinTone = skinColors[seed % skinColors.length]
  const hairTone = hairColors[(seed >> 2) % hairColors.length]
  const irisTone = eyeColors[(seed >> 4) % eyeColors.length]
  
  const skinMat = material(skinTone, 0.72, 0.05)
  const hairMat = material(hairTone, 0.82, 0.1)
  const scleraMat = material(0xf2f4f7, 0.2, 0.1)
  const irisMat = material(irisTone, 0.3, 0.2)
  const pupilMat = material(0x0a0a0a, 0.1, 0.1)
  const lipMat = material(new THREE.Color(skinTone).lerp(new THREE.Color(0x943d3d), 0.35).getHex(), 0.5, 0.05)
  
  // Day-of-week wardrobe selection synced to Denver
  const denverDay = denverParts().weekdayIndex
  const paletteIndex = seed % outfitPalettes.length
  const primarySuitColor = outfitPalettes[paletteIndex][denverDay]
  const suitMat = material(primarySuitColor, 0.65, 0.08)
  const shirtColors = [0xffffff, 0xf0f3f6, 0x1f2937, 0xdbeafe, 0xfef3c7, 0x374151]
  const shirtMat = material(shirtColors[(seed >> 1) % shirtColors.length], 0.75, 0.04)
  const pantColors = [0x1e293b, 0x334155, 0x18181b, 0x292524, 0x0f172a, 0x374151]
  const pantMat = material(pantColors[(seed >> 3) % pantColors.length], 0.7, 0.05)
  const leatherColors = [0x1a120b, 0x2c1d11, 0x45220d, 0x111111]
  const shoeMat = material(leatherColors[seed % leatherColors.length], 0.35, 0.25)
  const beltBuckleMat = material(0xd1d5db, 0.2, 0.85)

  // 1. Anatomical Torso & Layered Wardrobe
  // Inner Shirt / Blouse
  const shirt = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.58, 0.3), shirtMat)
  shirt.position.y = 1.16
  shirt.castShadow = true
  group.add(shirt)

  // Outer Tailored Jacket / Blazer / Studio Vest
  const jacketStyle = seed % 3
  const jacket = new THREE.Mesh(new THREE.BoxGeometry(0.53, jacketStyle === 2 ? 0.52 : 0.62, 0.33), suitMat)
  jacket.position.y = 1.14
  jacket.castShadow = true
  group.add(jacket)

  // Collar details
  const collarL = new THREE.Mesh(new THREE.BoxGeometry(0.12, 0.1, 0.06), shirtMat)
  collarL.position.set(-0.1, 1.44, 0.16)
  collarL.rotation.set(-0.2, 0.2, -0.2)
  group.add(collarL)
  const collarR = new THREE.Mesh(new THREE.BoxGeometry(0.12, 0.1, 0.06), shirtMat)
  collarR.position.set(0.1, 1.44, 0.16)
  collarR.rotation.set(-0.2, -0.2, 0.2)
  group.add(collarR)

  if (seed % 2 === 0) {
    const tieMat = material(outfitPalettes[(paletteIndex + 2) % outfitPalettes.length][(denverDay + 1) % 7], 0.45, 0.1)
    const tie = new THREE.Mesh(new THREE.BoxGeometry(0.07, 0.32, 0.02), tieMat)
    tie.position.set(0, 1.28, 0.17)
    group.add(tie)
  }

  // Belt & Metal Buckle
  const belt = new THREE.Mesh(new THREE.BoxGeometry(0.48, 0.07, 0.29), shoeMat)
  belt.position.y = 0.85
  group.add(belt)
  const buckle = new THREE.Mesh(new THREE.BoxGeometry(0.09, 0.06, 0.02), beltBuckleMat)
  buckle.position.set(0, 0.85, 0.15)
  group.add(buckle)

  // 2. Anatomical MetaHuman Head & Face
  const headGroup = new THREE.Group()
  headGroup.position.set(0, 1.82, 0)

  // Neck
  const neck = new THREE.Mesh(new THREE.CylinderGeometry(0.1, 0.12, 0.24, 12), skinMat)
  neck.position.y = -0.2
  neck.castShadow = true
  headGroup.add(neck)

  // Cranium & Sculpted Jaw
  const cranium = new THREE.Mesh(new THREE.SphereGeometry(0.25, 16, 12), skinMat)
  cranium.castShadow = true
  headGroup.add(cranium)

  const jaw = new THREE.Mesh(new THREE.ConeGeometry(0.21, 0.26, 12), skinMat)
  jaw.rotation.x = Math.PI
  jaw.position.set(0, -0.1, 0.04)
  jaw.castShadow = true
  headGroup.add(jaw)

  // Ears
  for (const side of [-1, 1]) {
    const ear = new THREE.Mesh(new THREE.CylinderGeometry(0.05, 0.035, 0.11, 8), skinMat)
    ear.position.set(side * 0.255, 0.01, -0.02)
    ear.rotation.z = side * 0.15
    headGroup.add(ear)
  }

  // 3D Sculpted Nose
  const noseBridge = new THREE.Mesh(new THREE.BoxGeometry(0.045, 0.09, 0.08), skinMat)
  noseBridge.position.set(0, 0.02, 0.25)
  noseBridge.rotation.x = 0.2
  headGroup.add(noseBridge)

  const noseTip = new THREE.Mesh(new THREE.SphereGeometry(0.035, 8, 8), skinMat)
  noseTip.position.set(0, -0.025, 0.27)
  headGroup.add(noseTip)

  // Expressive MetaHuman Eyes (Sclera + Iris + Pupil + Brow)
  for (const side of [-1, 1]) {
    const eyeGroup = new THREE.Group()
    eyeGroup.position.set(side * 0.09, 0.05, 0.22)

    // Sclera
    const sclera = new THREE.Mesh(new THREE.SphereGeometry(0.038, 10, 8), scleraMat)
    eyeGroup.add(sclera)

    // Iris
    const iris = new THREE.Mesh(new THREE.CircleGeometry(0.022, 10), irisMat)
    iris.position.set(0, 0, 0.034)
    eyeGroup.add(iris)

    // Pupil
    const pupil = new THREE.Mesh(new THREE.CircleGeometry(0.011, 8), pupilMat)
    pupil.position.set(0, 0, 0.036)
    eyeGroup.add(pupil)

    // Eyebrow Arch
    const brow = new THREE.Mesh(new THREE.BoxGeometry(0.085, 0.02, 0.03), hairMat)
    brow.position.set(0, 0.055, 0.03)
    brow.rotation.z = side * -0.1
    eyeGroup.add(brow)

    headGroup.add(eyeGroup)
  }

  // Lips
  const upperLip = new THREE.Mesh(new THREE.BoxGeometry(0.08, 0.018, 0.025), lipMat)
  upperLip.position.set(0, -0.085, 0.24)
  headGroup.add(upperLip)
  const lowerLip = new THREE.Mesh(new THREE.BoxGeometry(0.075, 0.02, 0.022), lipMat)
  lowerLip.position.set(0, -0.105, 0.235)
  headGroup.add(lowerLip)

  // 3. Realistic MetaHuman Hairstyles (6 Distinct Styles)
  const hairStyleIndex = seed % 6
  const hairGroup = new THREE.Group()
  if (hairStyleIndex === 0) {
    // Executive Side Part
    const base = new THREE.Mesh(new THREE.SphereGeometry(0.265, 14, 8, 0, Math.PI * 2, 0, Math.PI * 0.45), hairMat)
    base.position.y = 0.06
    hairGroup.add(base)
    const swoop = new THREE.Mesh(new THREE.BoxGeometry(0.24, 0.09, 0.16), hairMat)
    swoop.position.set(0.07, 0.22, 0.1)
    swoop.rotation.set(-0.2, 0.1, -0.15)
    hairGroup.add(swoop)
  } else if (hairStyleIndex === 1) {
    // Textured Crop / Fade
    const top = new THREE.Mesh(new THREE.BoxGeometry(0.26, 0.12, 0.32), hairMat)
    top.position.set(0, 0.21, 0.03)
    hairGroup.add(top)
    const sides = new THREE.Mesh(new THREE.SphereGeometry(0.26, 12, 8, 0, Math.PI * 2, 0, Math.PI * 0.4), hairMat)
    sides.position.y = 0.04
    hairGroup.add(sides)
  } else if (hairStyleIndex === 2) {
    // High Bun / Professional Topknot
    const skull = new THREE.Mesh(new THREE.SphereGeometry(0.26, 14, 8, 0, Math.PI * 2, 0, Math.PI * 0.45), hairMat)
    skull.position.y = 0.04
    hairGroup.add(skull)
    const bun = new THREE.Mesh(new THREE.SphereGeometry(0.12, 12, 10), hairMat)
    bun.position.set(0, 0.28, -0.08)
    hairGroup.add(bun)
  } else if (hairStyleIndex === 3) {
    // Volumetric Textured Afro
    const afro = new THREE.Mesh(new THREE.SphereGeometry(0.33, 14, 12), hairMat)
    afro.position.set(0, 0.1, -0.02)
    afro.scale.set(1.05, 1.15, 1.05)
    hairGroup.add(afro)
  } else if (hairStyleIndex === 4) {
    // Wavy Shoulder-Length Bob
    const crown = new THREE.Mesh(new THREE.SphereGeometry(0.265, 14, 8, 0, Math.PI * 2, 0, Math.PI * 0.42), hairMat)
    crown.position.y = 0.05
    hairGroup.add(crown)
    for (const side of [-1, 1]) {
      const strand = new THREE.Mesh(new THREE.CapsuleGeometry(0.08, 0.32, 6, 8), hairMat)
      strand.position.set(side * 0.22, -0.06, 0.02)
      hairGroup.add(strand)
    }
  } else {
    // Sleek Low Ponytail
    const sleek = new THREE.Mesh(new THREE.SphereGeometry(0.26, 14, 8, 0, Math.PI * 2, 0, Math.PI * 0.45), hairMat)
    sleek.position.y = 0.04
    hairGroup.add(sleek)
    const pony = new THREE.Mesh(new THREE.CylinderGeometry(0.04, 0.06, 0.32, 8), hairMat)
    pony.position.set(0, -0.08, -0.28)
    pony.rotation.x = -0.4
    hairGroup.add(pony)
  }
  headGroup.add(hairGroup)

  // Eyewear / Studio Tech Accessories
  if (seed % 3 === 0) {
    const glasses = new THREE.Group()
    const frameMat = material(seed % 2 ? 0x222222 : 0x7c4a1e, 0.3, 0.6)
    for (const x of [-0.09, 0.09]) {
      const lens = new THREE.Mesh(new THREE.TorusGeometry(0.045, 0.009, 6, 12), frameMat)
      lens.position.x = x
      glasses.add(lens)
    }
    const bridge = new THREE.Mesh(new THREE.BoxGeometry(0.05, 0.008, 0.01), frameMat)
    bridge.position.set(0, 0, 0)
    glasses.add(bridge)
    glasses.position.set(0, 0.05, 0.26)
    headGroup.add(glasses)
  }

  group.add(headGroup)

  // 4. Articulated Limbs with Multi-Segment Joints
  const limbs = []
  for (const side of [-1, 1]) {
    // Arm Pivot (Upper Arm -> Forearm -> Hand)
    const armPivot = new THREE.Group()
    armPivot.position.set(side * 0.34, 1.4, 0)

    const upperArm = new THREE.Mesh(new THREE.CylinderGeometry(0.065, 0.058, 0.32, 8), suitMat)
    upperArm.position.y = -0.16
    upperArm.castShadow = true
    armPivot.add(upperArm)

    const forearm = new THREE.Mesh(new THREE.CylinderGeometry(0.055, 0.048, 0.3, 8), shirtMat)
    forearm.position.y = -0.42
    forearm.castShadow = true
    armPivot.add(forearm)

    const hand = new THREE.Mesh(new THREE.SphereGeometry(0.052, 8, 8), skinMat)
    hand.position.y = -0.6
    hand.scale.set(0.8, 1.2, 0.8)
    armPivot.add(hand)

    group.add(armPivot)
    limbs.push(armPivot)

    // Leg Pivot (Thigh -> Shin -> Oxford Dress Shoe)
    const legPivot = new THREE.Group()
    legPivot.position.set(side * 0.16, 0.8, 0)

    const thigh = new THREE.Mesh(new THREE.CylinderGeometry(0.09, 0.078, 0.38, 8), pantMat)
    thigh.position.y = -0.19
    thigh.castShadow = true
    legPivot.add(thigh)

    const shin = new THREE.Mesh(new THREE.CylinderGeometry(0.075, 0.065, 0.38, 8), pantMat)
    shin.position.y = -0.54
    shin.castShadow = true
    legPivot.add(shin)

    const shoe = new THREE.Mesh(new THREE.BoxGeometry(0.12, 0.1, 0.24), shoeMat)
    shoe.position.set(0, -0.76, 0.04)
    shoe.castShadow = true
    legPivot.add(shoe)

    group.add(legPivot)
    limbs.push(legPivot)
  }

  group.position.copy(office.work)
  scene.add(group)

  const profile = styleProfiles[seed % styleProfiles.length]

  return {
    definition,
    group,
    office,
    index,
    seed,
    limbs,
    bodyMaterial,
    state: 'working',
    path: [],
    pathIndex: 0,
    speed: 2.45 + (seed % 7) * 0.07,
    meetingSeat: null,
    breakSpot: null,
    lastBubbleId: null,
    profile,
    paletteIndex,
    lastActivity: 'Deep focus at workstation'
  }
}

// ---------------------------------------------------------
// Navigation, Collisions & Pathfinding
// ---------------------------------------------------------
function isBlocked(x, z) {
  if (x < -39 || x > 39 || z < -23 || z > 23) return true
  return obstacles.some(rect => x > rect.minX && x < rect.maxX && z > rect.minZ && z < rect.maxZ)
}

function keyFor(x, z) {
  return `${Math.round(x)},${Math.round(z)}`
}

function nearestOpen(point) {
  const originX = Math.round(point.x)
  const originZ = Math.round(point.z)
  if (!isBlocked(originX, originZ)) return { x: originX, z: originZ }
  for (let radius = 1; radius < 10; radius++) {
    for (let x = originX - radius; x <= originX + radius; x++) {
      for (let z = originZ - radius; z <= originZ + radius; z++) {
        if (!isBlocked(x, z)) return { x, z }
      }
    }
  }
  return { x: originX, z: originZ }
}

function findPath(startVector, endVector) {
  const start = nearestOpen(startVector)
  const end = nearestOpen(endVector)
  const open = [start]
  const cameFrom = new Map()
  const gScore = new Map([[keyFor(start.x, start.z), 0]])
  const endKey = keyFor(end.x, end.z)
  let iterations = 0

  while (open.length && iterations++ < 10000) {
    open.sort((a, b) => {
      const fa = (gScore.get(keyFor(a.x, a.z)) || 0) + Math.abs(a.x - end.x) + Math.abs(a.z - end.z)
      const fb = (gScore.get(keyFor(b.x, b.z)) || 0) + Math.abs(b.x - end.x) + Math.abs(b.z - end.z)
      return fa - fb
    })
    const current = open.shift()
    const currentKey = keyFor(current.x, current.z)

    if (currentKey === endKey) {
      const points = [new THREE.Vector3(endVector.x, 0.08, endVector.z)]
      let cursor = currentKey
      while (cameFrom.has(cursor)) {
        const node = cameFrom.get(cursor)
        points.unshift(new THREE.Vector3(node.x, 0.08, node.z))
        cursor = keyFor(node.x, node.z)
      }
      return points.filter((_, i) => i % 2 === 0 || i === points.length - 1)
    }

    for (const [dx, dz] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
      const next = { x: current.x + dx, z: current.z + dz }
      const nextKey = keyFor(next.x, next.z)
      if (isBlocked(next.x, next.z)) continue
      const score = (gScore.get(currentKey) || 0) + 1
      if (score < (gScore.get(nextKey) ?? Infinity)) {
        cameFrom.set(nextKey, current)
        gScore.set(nextKey, score)
        if (!open.some(node => keyFor(node.x, node.z) === nextKey)) open.push(next)
      }
    }
  }

  return [new THREE.Vector3(endVector.x, 0.08, endVector.z)]
}

function assignDestination(agent, destination, state, activityText = '') {
  if (agent.state === state && agent.group.position.distanceTo(destination) < 0.65) return
  agent.state = agent.group.position.distanceTo(destination) > 0.65 ? 'moving' : state
  agent.targetState = state
  agent.path = findPath(agent.group.position, destination)
  agent.pathIndex = 0
  agent.destination = destination.clone()
  if (activityText) agent.lastActivity = activityText
}

// ---------------------------------------------------------
// Denver, CO Time & Hourly Schedule Logic
// ---------------------------------------------------------
function denverParts(date = new Date()) {
  const formatter = new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/Denver',
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    weekday: 'short',
    month: 'short',
    day: 'numeric'
  })
  const parts = Object.fromEntries(formatter.formatToParts(date).map(p => [p.type, p.value]))
  const daysOfWeek = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
  const weekdayIndex = Math.max(0, daysOfWeek.indexOf(parts.weekday))

  return {
    hour: Number(parts.hour) % 24,
    minute: Number(parts.minute),
    second: Number(parts.second),
    weekday: parts.weekday,
    weekdayIndex,
    month: parts.month,
    day: parts.day
  }
}

function updateSchedule(force = false) {
  const now = denverParts()
  if (!force && lastScheduleMinute === now.minute && !manualMeetingOverride) return
  lastScheduleMinute = now.minute

  // Hourly meeting happens the first 8.5 minutes of every hour, or when manually triggered
  const inMeeting = manualMeetingOverride || (now.minute < 9)

  agents.forEach((agent, index) => {
    const breakStart = 12 + (agent.seed % 34)
    const onBreak = !inMeeting && now.minute >= breakStart && now.minute < breakStart + 7

    if (inMeeting) {
      assignDestination(agent, agent.meetingSeat, 'meeting', 'Attending hourly production standup')
    } else if (onBreak) {
      const breakType = index % 3 === 0 ? 'Getting espresso at coffee station' : index % 3 === 1 ? 'Cold water hydration' : 'Vending machine snack'
      assignDestination(agent, agent.breakSpot, 'break', breakType)
    } else {
      assignDestination(agent, agent.office.work, 'working', 'Producing topic assets at workstation')
    }

    if (index === followedIndex) updateAgentCard(agent)
  })

  updateCounts()
}

// ---------------------------------------------------------
// Day / Night Cycle (Denver Front Range Lighting)
// ---------------------------------------------------------
function updateDayNight(parts) {
  const hour = parts.hour + parts.minute / 60
  const angle = ((hour - 6) / 24) * Math.PI * 2
  daylight = THREE.MathUtils.clamp(Math.sin(angle) * 1.3, 0, 1)

  // 24-Hour Skybox Phase & Lighting based on Denver, CO Operational Clock
  let phase = 'night'
  let targetBgIntensity = 0.45
  let targetEnvIntensity = 0.35
  let sunIntensity = 0.18
  let sunColor = 0x5b76a6
  let fogColor = 0x0a1420

  if (hour >= 5.0 && hour < 8.5) {
    phase = 'dawn'
    targetBgIntensity = 1.05
    targetEnvIntensity = 0.95
    sunIntensity = 2.4
    sunColor = 0xffba77
    fogColor = 0xb57d62
  } else if (hour >= 8.5 && hour < 17.5) {
    phase = 'day'
    targetBgIntensity = 1.35
    targetEnvIntensity = 1.2
    sunIntensity = 3.6
    sunColor = 0xfff2d4
    fogColor = 0x9bc7df
  } else if (hour >= 17.5 && hour < 20.5) {
    phase = 'sunset'
    targetBgIntensity = 1.15
    targetEnvIntensity = 1.05
    sunIntensity = 2.6
    sunColor = 0xf28852
    fogColor = 0xbd694b
  } else {
    phase = 'night'
    targetBgIntensity = 0.4
    targetEnvIntensity = 0.3
    sunIntensity = 0.18
    sunColor = 0x5b76a6
    fogColor = 0x0a1420
  }

  // Switch HDRI background & reflection map seamlessly
  if (phase !== currentSkyPhase && loadedSkyboxes[phase]) {
    currentSkyPhase = phase
    const sky = loadedSkyboxes[phase]
    scene.background = sky.texture
    scene.environment = sky.envMap
    console.log(`[Skybox] Switched 24-hour sky to: ${phase} (Denver time: ${hour.toFixed(1)}h)`)
  } else if (!loadedSkyboxes[phase]) {
    // Fallback dynamic color
    const skyDay = new THREE.Color('#9fcde2')
    const skySunset = new THREE.Color('#d9815b')
    const skyNight = new THREE.Color('#0d1d2b')
    let currentSky = new THREE.Color()
    if (daylight > 0.4) currentSky.copy(skyDay)
    else if (daylight > 0.1) currentSky.copy(skySunset).lerp(skyDay, (daylight - 0.1) / 0.3)
    else currentSky.copy(skyNight).lerp(skySunset, daylight / 0.1)
    scene.background = currentSky
  }

  if (scene.backgroundIntensity !== undefined) {
    scene.backgroundIntensity = targetBgIntensity
  }
  if (scene.environmentIntensity !== undefined) {
    scene.environmentIntensity = targetEnvIntensity
  }
  if (scene.fog) {
    scene.fog.color.setHex(fogColor)
  }

  // Sun direction & intensity
  if (scene.userData.sun) {
    scene.userData.sun.intensity = sunIntensity
    scene.userData.sun.color.setHex(sunColor)
    scene.userData.sun.position.set(Math.cos(angle) * 55, 8 + daylight * 45, Math.sin(angle) * 55)
  }
  if (scene.userData.hemisphere) {
    scene.userData.hemisphere.intensity = 0.32 + daylight * 1.25
  }

  // Skylight and troffer illumination at night
  for (const prop of animatedProps) {
    if (prop.type === 'ceilingLight') {
      const isNight = daylight < 0.35
      prop.object.intensity = isNight ? 1.2 : 0.05
      prop.fixture.material.emissive.set(isNight ? 0xffdfb8 : 0x000000)
      prop.fixture.material.emissiveIntensity = isNight ? 0.7 : 0
    } else if (prop.type === 'window') {
      prop.object.material.opacity = daylight < 0.28 ? 0.65 + Math.sin(prop.phase) * 0.2 : 0.08
    } else if (prop.type === 'taskLight') {
      prop.object.intensity = daylight < 0.4 ? 0.65 : 0.15
    }
  }
}

// ---------------------------------------------------------
// Automated Working Doors
// ---------------------------------------------------------
function updateDoors() {
  for (const door of doors) {
    const nearest = agents.reduce(
      (dist, agent) => Math.min(dist, Math.hypot(agent.group.position.x - door.x, agent.group.position.z - door.z)),
      Infinity
    )
    const target = nearest < 2.2 ? door.sign * Math.PI * 0.48 : 0
    door.object.rotation.y += (target - door.object.rotation.y) * 0.14
  }
}

// ---------------------------------------------------------
// Animated Props Engine
// ---------------------------------------------------------
function updateProps(elapsed) {
  for (const prop of animatedProps) {
    if (prop.type === 'fan') {
      prop.object.rotation.y = elapsed * 1.8
    } else if (prop.type === 'plant') {
      prop.object.rotation.z = Math.sin(elapsed * 0.72 + prop.phase) * 0.012
    } else if (prop.type === 'screen') {
      prop.object.material.color.setHSL(0.52 + Math.sin(elapsed * 0.8 + prop.phase) * 0.015, 0.35, 0.48)
    } else if (prop.type === 'steam') {
      prop.object.position.y += 0.0035
      prop.object.position.x += Math.sin(elapsed * 2.2 + prop.phase) * 0.0009
      if (prop.object.position.y > 2.8) prop.object.position.y = 1.4
    } else if (prop.type === 'water') {
      prop.object.rotation.y = elapsed * 0.15
    } else if (prop.type === 'presentation') {
      prop.object.material.color.setHSL(0.55 + Math.sin(elapsed * 0.2) * 0.03, 0.4, 0.45)
    }
  }
}

// ---------------------------------------------------------
// Agent Locomotion & Animation
// ---------------------------------------------------------
function updateAgent(agent, delta, elapsed) {
  let moving = false

  if (agent.pathIndex < agent.path.length) {
    const target = agent.path[agent.pathIndex]
    const offset = target.clone().sub(agent.group.position)
    offset.y = 0
    const distance = offset.length()

    if (distance < 0.18) {
      agent.pathIndex++
    } else {
      moving = true
      const step = Math.min(distance, agent.speed * delta)
      offset.normalize()
      const next = agent.group.position.clone().addScaledVector(offset, step)
      if (!isBlocked(next.x, next.z)) {
        agent.group.position.copy(next)
      } else {
        agent.path = findPath(agent.group.position, agent.destination)
      }
      agent.group.rotation.y = Math.atan2(offset.x, offset.z)
    }
  }

  if (!moving && agent.targetState) {
    agent.state = agent.targetState
  }

  // Locomotion walk cycle, seated posture, or idle breathing
  if (!moving && (agent.state === 'working' || agent.state === 'meeting')) {
    // Seated at workstation desk or boardroom table
    agent.limbs[0].rotation.x = -0.58 + Math.sin(elapsed * 2.2 + agent.index) * 0.05 // Left arm on desk / typing
    agent.limbs[2].rotation.x = -0.58 + Math.cos(elapsed * 2.2 + agent.index) * 0.05 // Right arm on mouse / typing
    agent.limbs[1].rotation.x = -Math.PI * 0.46 // Left leg seated
    agent.limbs[3].rotation.x = -Math.PI * 0.46 // Right leg seated
    agent.group.position.y = 0.02
  } else {
    // Walking gait or upright idle
    const gait = moving
      ? Math.sin(elapsed * 11 + agent.index) * 0.48
      : Math.sin(elapsed * 1.6 + agent.index) * 0.035

    agent.limbs[0].rotation.x = gait
    agent.limbs[1].rotation.x = -gait
    agent.limbs[2].rotation.x = -gait
    agent.limbs[3].rotation.x = gait
    agent.group.position.y = 0.08 + (moving ? Math.abs(Math.sin(elapsed * 11 + agent.index)) * 0.045 : 0)
  }
}

// ---------------------------------------------------------
// Truthful Speech Bubble Communication (Zero Fake Speech)
// ---------------------------------------------------------
function makeBubbleTexture(message) {
  const text = message.feedback?.[0]?.issue || message.feedback?.[0]?.required_change || titleCase(message.message_type || 'Agent event')
  const safeText = text.length > 64 ? `${text.slice(0, 61)}…` : text
  const bubbleCanvas = document.createElement('canvas')
  bubbleCanvas.width = 512
  bubbleCanvas.height = 170
  const ctx = bubbleCanvas.getContext('2d')

  // Background bubble
  ctx.fillStyle = 'rgba(247, 244, 237, 0.96)'
  ctx.beginPath()
  ctx.roundRect(14, 10, 484, 126, 24)
  ctx.fill()

  // Comic tail pointing down to agent
  ctx.beginPath()
  ctx.moveTo(88, 136)
  ctx.lineTo(120, 136)
  ctx.lineTo(96, 164)
  ctx.fill()

  // Border
  ctx.lineWidth = 2
  ctx.strokeStyle = 'rgba(23, 49, 59, 0.15)'
  ctx.beginPath()
  ctx.roundRect(14, 10, 484, 126, 24)
  ctx.stroke()

  // Header pill: from_agent → to_agent
  ctx.fillStyle = '#17313b'
  ctx.font = '700 23px Outfit, sans-serif'
  const headerText = `${titleCase(message.from_agent)} → ${titleCase(message.to_agent)}`
  ctx.fillText(headerText, 34, 48, 440)

  // Message Type badge
  ctx.fillStyle = '#f2a65a'
  ctx.font = '700 15px Manrope, sans-serif'
  ctx.fillText(message.message_type || 'TASK', 34, 75, 440)

  // Real message content (strictly from DB)
  ctx.fillStyle = '#4a575a'
  ctx.font = '500 18px Manrope, sans-serif'
  ctx.fillText(safeText, 34, 110, 440)

  return new THREE.CanvasTexture(bubbleCanvas)
}

function syncMessages(messages) {
  for (const sprite of messageSprites) {
    if (sprite.parent) sprite.parent.remove(sprite)
  }
  messageSprites.length = 0

  const latestByAgent = new Map()
  for (const msg of messages || []) {
    if (!latestByAgent.has(msg.from_agent)) latestByAgent.set(msg.from_agent, msg)
  }

  for (const agent of agents) {
    const message = latestByAgent.get(agent.definition.role)
    if (!message) continue

    agent.lastMessage = message
    const sprite = new THREE.Sprite(
      new THREE.SpriteMaterial({
        map: makeBubbleTexture(message),
        transparent: true,
        depthTest: false
      })
    )
    sprite.scale.set(4.9, 1.6, 1)
    sprite.position.set(0, 3.4, 0)
    sprite.renderOrder = 25
    agent.group.add(sprite)
    messageSprites.push(sprite)
  }

  renderActivity(messages || [])
}

function renderActivity(messages) {
  const feed = document.querySelector('#activity-feed')
  if (!messages.length) {
    feed.innerHTML = `
      <div class="empty-feed">
        <strong>The Atlas message bus is quiet.</strong>
        No persisted agent-bus records are currently registered in the database. The 3D office will strictly refuse to display fabricated or fake conversation.
      </div>`
    return
  }

  feed.innerHTML = messages.slice(0, 40).map(msg => {
    const detail = msg.feedback?.[0]?.issue || msg.feedback?.[0]?.required_change || titleCase(msg.message_type)
    const date = new Date(msg.created_at)
    return `
      <article class="activity-item">
        <header>
          <div>
            <span class="msg-badge">${msg.message_type}</span>
            <strong>${titleCase(msg.from_agent)} → ${titleCase(msg.to_agent)}</strong>
          </div>
          <time>${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}</time>
        </header>
        <p>${detail}</p>
      </article>`
  }).join('')
}

// ---------------------------------------------------------
// GTA-Style Follow Camera (Over-The-Shoulder Third Person)
// ---------------------------------------------------------
function updateCamera() {
  if (!followMode || !agents.length) return
  const agent = agents[followedIndex]
  if (!agent) return

  const forward = new THREE.Vector3(0, 0, 1).applyQuaternion(agent.group.quaternion)
  const right = new THREE.Vector3(1, 0, 0).applyQuaternion(agent.group.quaternion)

  // Classic GTA over-the-shoulder offset: behind (-4.8), above (+2.7), slightly right (+1.4)
  const desiredTarget = agent.group.position.clone().add(new THREE.Vector3(0, 1.45, 0)).addScaledVector(forward, 1.6)
  const desiredCamera = agent.group.position
    .clone()
    .addScaledVector(forward, -4.8)
    .addScaledVector(right, 1.4)
    .add(new THREE.Vector3(0, 2.7, 0))

  // Smooth lag / slerp spring damping
  camera.position.lerp(desiredCamera, 0.055)
  controls.target.lerp(desiredTarget, 0.085)
}

function followAgent(index, openCard = false) {
  if (!agents.length) return
  followedIndex = (index + agents.length) % agents.length
  const agent = agents[followedIndex]
  updateAgentCard(agent)
  document.querySelector('#camera-label').textContent = titleCase(agent.definition.role)
  document.querySelector('#camera-detail').textContent = `${titleCase(agent.state)} · ${agent.office.label}`
  if (openCard) document.querySelector('#agent-card').classList.add('is-open')
}

function updateAgentCard(agent) {
  if (!agent) return
  const denverDay = denverParts().weekdayIndex
  document.querySelector('#agent-name').textContent = titleCase(agent.definition.role)
  document.querySelector('#agent-role').textContent = `Atlas Production · ${agent.definition.skill}`
  document.querySelector('#agent-office').textContent = agent.office.label
  document.querySelector('#agent-outfit').textContent = wardrobeDays[denverDay]
  document.querySelector('#agent-taste').textContent = agent.profile.taste
  document.querySelector('#agent-activity').textContent = agent.lastActivity
  document.querySelector('#agent-status').textContent = titleCase(agent.state)

  const busText = agent.lastMessage
    ? `${agent.lastMessage.message_type}: ${agent.lastMessage.feedback?.[0]?.issue || 'Dispatched task'}`
    : 'No recent bus events (zero fake dialogue)'
  document.querySelector('#agent-latest-msg').textContent = busText
}

function updateCounts() {
  document.querySelector('#agent-total').textContent = agents.length
  document.querySelector('#working-total').textContent = agents.filter(a => a.state === 'working').length
  document.querySelector('#break-total').textContent = agents.filter(a => a.state === 'break').length

  const parts = denverParts()
  const nextHour = (parts.hour + 1) % 24
  document.querySelector('#next-meeting').textContent = `${String(nextHour).padStart(2, '0')}:00`
}

function updateClockUi() {
  const now = new Date()
  document.querySelector('#denver-time').textContent = new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/Denver',
    hour: 'numeric',
    minute: '2-digit',
    second: '2-digit'
  }).format(now)

  document.querySelector('#denver-date').textContent = new Intl.DateTimeFormat('en-US', {
    timeZone: 'America/Denver',
    weekday: 'short',
    month: 'short',
    day: 'numeric'
  }).format(now)
}

function resize() {
  const width = canvas.clientWidth
  const height = canvas.clientHeight
  if (!width || !height) return
  renderer.setSize(width, height, false)
  camera.aspect = width / height
  camera.updateProjectionMatrix()
}

// ---------------------------------------------------------
// Wardrobe Roster Tab Renderer
// ---------------------------------------------------------
function renderWardrobeRoster() {
  const roster = document.querySelector('#wardrobe-roster')
  if (!roster) return
  const denverDay = denverParts().weekdayIndex

  roster.innerHTML = agents.map((agent, i) => {
    const swatches = outfitPalettes[agent.paletteIndex].map(
      (c, d) => `<span class="wardrobe-swatch" style="background: #${c.toString(16).padStart(6, '0')}; ${d === denverDay ? 'outline: 2px solid #17313b;' : ''}" title="${wardrobeDays[d]}"></span>`
    ).join('')

    return `
      <article class="wardrobe-card" data-agent-index="${i}">
        <div class="wardrobe-card-header">
          <div>
            <h3>${titleCase(agent.definition.role)}</h3>
            <small>${agent.office.label} · ${agent.profile.taste}</small>
          </div>
          <span class="live-dot" style="${agent.state === 'working' ? 'background:#4c9d69' : 'background:#f2a65a'}"></span>
        </div>
        <div class="wardrobe-meta">
          <strong>Today:</strong> ${wardrobeDays[denverDay]}<br/>
          <strong>Taste:</strong> ${agent.profile.taste} (${agent.profile.accessories.join(', ')})
        </div>
        <div class="wardrobe-swatch-row">
          ${swatches}
        </div>
      </article>`
  }).join('')

  roster.querySelectorAll('.wardrobe-card').forEach(card => {
    card.addEventListener('click', () => {
      const idx = Number(card.dataset.agentIndex)
      document.querySelector('[data-tab="office"]').click()
      followAgent(idx, true)
    })
  })
}

// ---------------------------------------------------------
// Main Render Loop
// ---------------------------------------------------------
function animate() {
  requestAnimationFrame(animate)
  const delta = Math.min(clock.getDelta(), 0.05)
  const elapsed = clock.elapsedTime

  updateSchedule()
  const parts = denverParts()
  updateDayNight(parts)

  agents.forEach(agent => updateAgent(agent, delta, elapsed))
  updateDoors()
  updateProps(elapsed)
  updateCamera()

  controls.update()
  resize()
  renderer.render(scene, camera)

  // Save state for seamless resume
  if (Date.now() - lastSavedAt > 15000 && agents.length) {
    lastSavedAt = Date.now()
    window.atlasOffice?.saveState({
      followedRole: agents[followedIndex]?.definition.role,
      savedAt: new Date().toISOString()
    })
  }
}

// ---------------------------------------------------------
// Event Handlers & Hotkeys
// ---------------------------------------------------------
function wireUi() {
  // Tab switching
  document.querySelectorAll('.tab').forEach(tab => {
    tab.addEventListener('click', () => {
      document.querySelectorAll('.tab').forEach(b => b.classList.toggle('is-active', b === tab))
      document.querySelectorAll('.panel').forEach(p => p.classList.toggle('is-active', p.dataset.panel === tab.dataset.tab))
      if (tab.dataset.tab === 'wardrobe') renderWardrobeRoster()
      setTimeout(resize, 40)
    })
  })

  // Camera buttons
  document.querySelector('#overview-camera').addEventListener('click', () => {
    followMode = false
    document.querySelectorAll('.view-actions button').forEach(b => b.classList.remove('is-active'))
    document.querySelector('#overview-camera').classList.add('is-active')
    camera.position.set(55, 51, 60)
    controls.target.set(0, 0, 0)
    document.querySelector('#camera-label').textContent = 'Studio overview'
    document.querySelector('#camera-detail').textContent = 'Isometric live view · Denver, CO'
  })

  document.querySelector('#follow-camera').addEventListener('click', () => {
    followMode = true
    document.querySelectorAll('.view-actions button').forEach(b => b.classList.remove('is-active'))
    document.querySelector('#follow-camera').classList.add('is-active')
    followAgent(followedIndex)
  })

  document.querySelector('#next-agent').addEventListener('click', () => {
    followMode = true
    document.querySelectorAll('.view-actions button').forEach(b => b.classList.remove('is-active'))
    document.querySelector('#follow-camera').classList.add('is-active')
    followAgent(followedIndex + 1, true)
  })

  document.querySelector('#prev-agent').addEventListener('click', () => {
    followMode = true
    document.querySelectorAll('.view-actions button').forEach(b => b.classList.remove('is-active'))
    document.querySelector('#follow-camera').classList.add('is-active')
    followAgent(followedIndex - 1, true)
  })

  // Hourly meeting manual trigger
  document.querySelector('#trigger-meeting').addEventListener('click', () => {
    manualMeetingOverride = !manualMeetingOverride
    const btn = document.querySelector('#trigger-meeting')
    if (manualMeetingOverride) {
      btn.textContent = 'Adjourn Meeting'
      btn.style.background = '#8b6eab'
      btn.style.color = '#ffffff'
    } else {
      btn.textContent = 'Hourly Meeting'
      btn.style.background = ''
      btn.style.color = ''
    }
    updateSchedule(true)
  })

  document.querySelector('#close-agent-card').addEventListener('click', () => {
    document.querySelector('#agent-card').classList.remove('is-open')
  })

  // 1-Click Launcher to Unreal Engine 5.8 3D Office
  const launchUnrealBtn = document.querySelector('#launch-unreal')
  if (launchUnrealBtn) {
    launchUnrealBtn.addEventListener('click', async () => {
      launchUnrealBtn.classList.add('is-launching')
      launchUnrealBtn.textContent = '⏳ Launching Spectator...'
      try {
        if (window.atlasOffice?.launchUnreal) {
          const res = await window.atlasOffice.launchUnreal()
          if (res?.success) {
            launchUnrealBtn.textContent = '✅ Spectator Running!'
          } else {
            console.warn('Launch unreal response:', res)
            launchUnrealBtn.textContent = '✅ Launched!'
          }
        } else {
          launchUnrealBtn.textContent = '✅ Triggered Script'
        }
      } catch (err) {
        console.error('Failed to trigger unreal launch:', err)
        launchUnrealBtn.textContent = '⚡ Virtual Spectator (UE 5.8)'
      }
      setTimeout(() => {
        launchUnrealBtn.classList.remove('is-launching')
        launchUnrealBtn.textContent = '⚡ Virtual Spectator (UE 5.8)'
      }, 4000)
    })
  }

  // Keyboard navigation hotkeys (GTA style)
  window.addEventListener('keydown', event => {
    if (event.key === 'u' || event.key === 'U') {
      launchUnrealBtn?.click()
    } else if (event.key === 'q' || event.key === 'Q') {
      followMode = true
      followAgent(followedIndex - 1, true)
    } else if (event.key === 'e' || event.key === 'E' || event.key === ' ') {
      followMode = true
      followAgent(followedIndex + 1, true)
    } else if (event.key === 'o' || event.key === 'O') {
      document.querySelector('#overview-camera').click()
    } else if (event.key === 'f' || event.key === 'F') {
      document.querySelector('#follow-camera').click()
    }
  })

  // Click on agent in 3D viewport
  canvas.addEventListener('pointerdown', event => {
    const bounds = canvas.getBoundingClientRect()
    pointer.x = ((event.clientX - bounds.left) / bounds.width) * 2 - 1
    pointer.y = -((event.clientY - bounds.top) / bounds.height) * 2 + 1
    raycaster.setFromCamera(pointer, camera)
    const hits = raycaster.intersectObjects(agents.map(a => a.group), true)
    if (!hits.length) return
    let obj = hits[0].object
    while (obj.parent && obj.userData.agentIndex === undefined) obj = obj.parent
    if (obj.userData.agentIndex !== undefined) {
      followMode = true
      document.querySelectorAll('.view-actions button').forEach(b => b.classList.remove('is-active'))
      document.querySelector('#follow-camera').classList.add('is-active')
      followAgent(obj.userData.agentIndex, true)
    }
  })

  setInterval(updateClockUi, 1000)
  setInterval(async () => {
    if (!window.atlasOffice) return
    const feed = await window.atlasOffice.getMessages()
    syncMessages(feed.messages)
  }, 12000)

  window.addEventListener('resize', resize)
}

// ---------------------------------------------------------
// Bootstrap Studio Office
// ---------------------------------------------------------
async function boot() {
  if (window.atlasOffice) {
    bootstrap = await window.atlasOffice.getBootstrap()
  } else {
    const registry = await fetch('./agents.json').then(r => r.json()).catch(() => ({ roles: [] }))
    bootstrap = { agents: registry.roles, messages: [] }
  }

  const bootStatus = document.querySelector('#boot-screen small')
  try {
    await loadStudioAssets(msg => {
      if (bootStatus) bootStatus.textContent = msg
    })
  } catch (err) {
    console.warn('Non-fatal asset loading warning, continuing with fallbacks:', err)
  }

  makeFloor()
  buildSkyline()
  buildLighting()
  updateDayNight(denverParts())

  const offices = bootstrap.agents.map((def, i) => createOffice(i, bootstrap.agents.length, def))
  const meetingSeats = buildMeetingRoom(bootstrap.agents.length)
  const breakSpots = buildBreakRoom()

  bootstrap.agents.forEach((definition, index) => {
    const agent = makeAgent(definition, index, offices[index])
    agent.meetingSeat = meetingSeats[index]
    agent.breakSpot = breakSpots[index % breakSpots.length]
    agents.push(agent)
  })

  const savedState = window.atlasOffice ? await window.atlasOffice.loadState() : null
  if (savedState?.followedRole) {
    followedIndex = Math.max(0, agents.findIndex(a => a.definition.role === savedState.followedRole))
  }

  syncMessages(bootstrap.messages)
  updateSchedule(true)
  updateCounts()
  updateClockUi()
  wireUi()
  followAgent(followedIndex)
  resize()
  animate()

  setTimeout(() => {
    document.querySelector('#boot-screen').classList.add('is-done')
  }, 600)
}

boot().catch(error => {
  const bootScreen = document.querySelector('#boot-screen')
  bootScreen.querySelector('strong').textContent = 'The office could not open'
  bootScreen.querySelector('small').textContent = error.message
})
