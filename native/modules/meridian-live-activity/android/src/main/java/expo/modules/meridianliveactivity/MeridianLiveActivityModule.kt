package expo.modules.meridianliveactivity

import android.content.Intent
import android.os.Build
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class MeridianLiveActivityModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("MeridianLiveActivity")

        Function("isSupported") { true }

        AsyncFunction("start") { payload: Map<String, Any?> ->
            val context = appContext.reactContext ?: return@AsyncFunction
            val intent = TimerNotificationService.intent(context, payload)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        AsyncFunction("update") { payload: Map<String, Any?> ->
            val context = appContext.reactContext ?: return@AsyncFunction
            val intent = TimerNotificationService.intent(context, payload)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        AsyncFunction("end") {
            val context = appContext.reactContext ?: return@AsyncFunction
            context.stopService(Intent(context, TimerNotificationService::class.java))
        }

        Function("pushToken") { null as String? }
    }
}
