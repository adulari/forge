package dev.adulari.forge.spike

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.viewmodel.compose.viewModel

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
        )
        // `am start --es base http://10.0.2.2:7431/a1b2c3d4e5f60718293a4b5c6d7e8f90 --es session <id>` for scripted runs.
        val base = intent.getStringExtra("base") ?: "http://10.0.2.2:7431/a1b2c3d4e5f60718293a4b5c6d7e8f90"
        val openSession = intent.getStringExtra("session")
        setContent {
            val vm: AppViewModel = viewModel(factory = AppViewModel.factory(base))
            var open by remember { mutableStateOf(openSession) }
            Box(Modifier.fillMaxSize().background(T.bg0).statusBarsPadding()) {
                val id = open
                if (id == null) {
                    FleetScreen(vm, onOpen = { open = it })
                } else {
                    BackHandler { open = null }
                    SessionScreen(vm, id, onBack = { open = null })
                }
            }
        }
    }
}
