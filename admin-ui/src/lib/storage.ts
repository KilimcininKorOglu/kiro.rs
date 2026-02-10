const API_KEY_STORAGE_KEY = 'adminApiKey'
const DARK_MODE_STORAGE_KEY = 'darkMode'

export const storage = {
  getApiKey: () => localStorage.getItem(API_KEY_STORAGE_KEY),
  setApiKey: (key: string) => localStorage.setItem(API_KEY_STORAGE_KEY, key),
  removeApiKey: () => localStorage.removeItem(API_KEY_STORAGE_KEY),

  getDarkMode: () => localStorage.getItem(DARK_MODE_STORAGE_KEY) === 'true',
  setDarkMode: (enabled: boolean) => localStorage.setItem(DARK_MODE_STORAGE_KEY, String(enabled)),
}
