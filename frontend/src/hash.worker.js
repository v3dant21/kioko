self.onmessage = async ({ data }) => {
  try {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await data.arrayBuffer()))
    self.postMessage({ digest: Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('') })
  } catch (error) {
    self.postMessage({ error: error.message })
  }
}
