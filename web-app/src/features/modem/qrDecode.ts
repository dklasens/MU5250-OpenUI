// Decodes an eSIM QR code from an image the user picked, entirely in the browser: nothing is
// uploaded anywhere. Live camera scanning is not offered because browsers refuse the camera on a
// plain-http page.
//
// The native BarcodeDetector is used where the browser has one; otherwise jsQR is loaded on demand,
// so it never weighs on the rest of the dashboard. The decoded text is a secret (it carries the
// activation code): it is returned to the caller only, never logged.

/** Larger photos are scaled down before decoding; QR codes stay readable well below this. */
const MAX_SIDE = 1600

interface DetectedBarcode {
  rawValue: string
}
interface BarcodeDetectorLike {
  detect(source: ImageBitmapSource): Promise<DetectedBarcode[]>
}
type BarcodeDetectorCtor = new (opts: { formats: string[] }) => BarcodeDetectorLike

/** The QR code's text, or null when the image holds no readable QR code. */
export async function decodeQrImage(file: Blob): Promise<string | null> {
  const bitmap = await createImageBitmap(file)
  try {
    const Detector = (globalThis as { BarcodeDetector?: BarcodeDetectorCtor }).BarcodeDetector
    if (Detector) {
      try {
        const found = await new Detector({ formats: ['qr_code'] }).detect(bitmap)
        const text = found.find((b) => b.rawValue)?.rawValue
        if (text) return text
      } catch {
        // Unsupported format list or a detector failure: fall through to jsQR.
      }
    }
    const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height))
    const width = Math.max(1, Math.round(bitmap.width * scale))
    const height = Math.max(1, Math.round(bitmap.height * scale))
    const canvas = document.createElement('canvas')
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext('2d', { willReadFrequently: true })
    if (!ctx) return null
    ctx.drawImage(bitmap, 0, 0, width, height)
    const pixels = ctx.getImageData(0, 0, width, height)
    const { default: jsQR } = await import('jsqr')
    return jsQR(pixels.data, width, height, { inversionAttempts: 'attemptBoth' })?.data || null
  } finally {
    bitmap.close()
  }
}
