import {StrictMode} from 'react'
import {createRoot} from 'react-dom/client'
import './style.css'
import App from './App'
import {initI18n} from './i18n'
import {disableWebviewDefaults} from './lib/disableWebviewDefaults'
import {applyTheme, resolveTheme, systemPrefersDark} from './lib/theme'
import {initialState, loadSettings, useUi} from './stores/ui'

disableWebviewDefaults()

// The window stays hidden until App has rendered, so loading settings first causes no flash.
void loadSettings().then((settings) => {
    const state = initialState(settings)
    useUi.setState(state)
    applyTheme(resolveTheme(state.theme, systemPrefersDark()))
    initI18n(state.language)
    document.documentElement.lang = state.language

    createRoot(document.getElementById('root')!).render(
        <StrictMode>
            <App/>
        </StrictMode>,
    )
})
