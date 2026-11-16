import {StrictMode} from 'react'
import {createRoot} from 'react-dom/client'
import './style.css'
import App from './App'
import {initI18n} from './i18n'
import {disableWebviewDefaults} from './lib/disableWebviewDefaults'
import {applyTheme, resolveTheme, systemPrefersDark} from './lib/theme'
import {initialLanguage, initialTheme} from './stores/ui'

disableWebviewDefaults()
applyTheme(resolveTheme(initialTheme(), systemPrefersDark()))
const language = initialLanguage()
initI18n(language)
document.documentElement.lang = language

createRoot(document.getElementById('root')!).render(
    <StrictMode>
        <App/>
    </StrictMode>,
)
