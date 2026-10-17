import fs from 'fs'
import path from 'path'
import { fileURLToPath } from 'url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(__dirname, '..')
const MODELS_DIR = path.join(ROOT, 'public', 'assets', 'models')
const SKYBOX_DIR = path.join(ROOT, 'public', 'assets', 'skybox')

const MODELS = [
  'metal_office_desk',
  'classic_laptop',
  'modern_arm_chair_01',
  'CoffeeCart_01',
  'projector_screen',
  'potted_plant_01',
  'bar_chair_round_01',
  'modern_coffee_table_01'
]

const HDRIS = [
  { id: 'aarfontein_dawn_2', name: 'dawn' },
  { id: 'syferfontein_1d_clear', name: 'day' },
  { id: 'altanka', name: 'sunset' },
  { id: 'dikhololo_night', name: 'night' }
]

async function downloadFile(url, destPath) {
  if (fs.existsSync(destPath) && fs.statSync(destPath).size > 0) {
    console.log(`  [Cached] ${path.basename(destPath)}`)
    return
  }
  const dir = path.dirname(destPath)
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })

  console.log(`  [Downloading] ${url} -> ${path.relative(ROOT, destPath)}`)
  const res = await fetch(url)
  if (!res.ok) throw new Error(`HTTP ${res.status} downloading ${url}`)
  const buffer = await res.arrayBuffer()
  fs.writeFileSync(destPath, Buffer.from(buffer))
}

async function fetchModel(id) {
  console.log(`\n=== Processing Model: ${id} ===`)
  const targetDir = path.join(MODELS_DIR, id)
  if (!fs.existsSync(targetDir)) fs.mkdirSync(targetDir, { recursive: true })

  const metaUrl = `https://api.polyhaven.com/files/${id}`
  const metaRes = await fetch(metaUrl)
  if (!metaRes.ok) throw new Error(`Failed to fetch metadata for ${id}: HTTP ${metaRes.status}`)
  const data = await metaRes.json()

  const gltfInfo = data.gltf?.['1k'] || data.gltf?.['2k'] || Object.values(data.gltf || {})[0]
  if (!gltfInfo?.gltf) throw new Error(`No gltf found for ${id}`)

  const mainGltfUrl = gltfInfo.gltf.url
  const mainFilename = path.basename(new URL(mainGltfUrl).pathname)
  const mainDest = path.join(targetDir, mainFilename)

  await downloadFile(mainGltfUrl, mainDest)

  const includes = gltfInfo.gltf.include || {}
  for (const [relPath, incInfo] of Object.entries(includes)) {
    const incDest = path.join(targetDir, relPath)
    await downloadFile(incInfo.url, incDest)
  }

  // Also save a manifest for quick lookup
  const manifest = {
    id,
    gltfFile: mainFilename,
    relativePath: `/assets/models/${id}/${mainFilename}`
  }
  fs.writeFileSync(path.join(targetDir, 'asset.json'), JSON.stringify(manifest, null, 2))
  console.log(`✓ Model ${id} complete: ${manifest.relativePath}`)
}

async function fetchHdri({ id, name }) {
  console.log(`\n=== Processing HDRI: ${name} (${id}) ===`)
  const metaUrl = `https://api.polyhaven.com/files/${id}`
  const metaRes = await fetch(metaUrl)
  if (!metaRes.ok) throw new Error(`Failed to fetch metadata for ${id}: HTTP ${metaRes.status}`)
  const data = await metaRes.json()

  const hdrRes = data.hdri?.['1k'] || Object.values(data.hdri || {})[0]
  const hdrUrl = hdrRes?.hdr?.url || hdrRes?.exr?.url
  if (!hdrUrl) throw new Error(`No hdr url found for ${id}`)

  const destFile = path.join(SKYBOX_DIR, `${name}.hdr`)
  await downloadFile(hdrUrl, destFile)
  console.log(`✓ HDRI ${name} complete: /assets/skybox/${name}.hdr`)
}

async function run() {
  console.log('Starting Poly Haven asset downloader...')
  for (const model of MODELS) {
    try {
      await fetchModel(model)
    } catch (err) {
      console.error(`Failed to download model ${model}:`, err.message)
    }
  }

  for (const hdri of HDRIS) {
    try {
      await fetchHdri(hdri)
    } catch (err) {
      console.error(`Failed to download HDRI ${hdri.id}:`, err.message)
    }
  }
  console.log('\nAll assets successfully downloaded and ready for Three.js & Unreal Engine!')
}

run().catch(console.error)
