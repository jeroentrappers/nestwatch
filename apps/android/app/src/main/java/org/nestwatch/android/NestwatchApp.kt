// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android

import android.app.Application
import org.nestwatch.android.core.Nestwatch
import org.nestwatch.android.platform.Notifications

/** Creates the engine owner and the notification channels. */
class NestwatchApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Nestwatch.init(this)
        Notifications.createChannels(this)
    }
}
