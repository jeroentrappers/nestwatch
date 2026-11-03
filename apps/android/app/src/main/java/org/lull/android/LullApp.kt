// SPDX-License-Identifier: AGPL-3.0-or-later
package org.lull.android

import android.app.Application
import org.lull.android.core.Lull
import org.lull.android.platform.Notifications

/** Creates the engine owner and the notification channels. */
class LullApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Lull.init(this)
        Notifications.createChannels(this)
    }
}
