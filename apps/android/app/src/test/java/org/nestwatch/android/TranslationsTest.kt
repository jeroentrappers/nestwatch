// SPDX-License-Identifier: AGPL-3.0-or-later
package org.nestwatch.android

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/**
 * Every language of NFR-I18N-2 translates every string of the English
 * reference with the same placeholders, and its plurals have the
 * quantities the language needs (CLDR).
 */
class TranslationsTest {
    private val res = File("src/main/res").takeIf { it.isDirectory } ?: File("app/src/main/res")

    private val quantities = mapOf(
        "nl" to setOf("one", "other"),
        "de" to setOf("one", "other"),
        "sv" to setOf("one", "other"),
        "da" to setOf("one", "other"),
        "nb" to setOf("one", "other"),
        "fr" to setOf("one", "many", "other"),
        "es" to setOf("one", "many", "other"),
        "it" to setOf("one", "many", "other"),
        "pt-rBR" to setOf("one", "many", "other"),
        "pl" to setOf("one", "few", "many", "other"),
    )

    private class Strings(val strings: Map<String, String>, val plurals: Map<String, Map<String, String>>)

    private fun load(dir: String): Strings {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(File(res, "$dir/strings.xml"))
        val root = doc.documentElement
        val strings = LinkedHashMap<String, String>()
        val plurals = LinkedHashMap<String, Map<String, String>>()
        val kids = root.childNodes
        for (i in 0 until kids.length) {
            val e = kids.item(i) as? Element ?: continue
            when (e.tagName) {
                "string" -> strings[e.getAttribute("name")] = e.textContent
                "plurals" -> {
                    val items = LinkedHashMap<String, String>()
                    val its = e.getElementsByTagName("item")
                    for (j in 0 until its.length) {
                        val it = its.item(j) as Element
                        items[it.getAttribute("quantity")] = it.textContent
                    }
                    plurals[e.getAttribute("name")] = items
                }
            }
        }
        return Strings(strings, plurals)
    }

    /** Placeholders as "index:type" (unnumbered ones count 1, 2, ...), sorted; `%%` as "%". */
    private fun placeholders(s: String): List<String> {
        var next = 0
        return Regex("%(?:(\\d+)\\$)?([sd%])").findAll(s).map { m ->
            val type = m.groupValues[2]
            if (type == "%") "%" else "${m.groupValues[1].toIntOrNull() ?: ++next}:$type"
        }.sorted().toList()
    }

    @Test
    fun everyLanguageMatchesEnglish() {
        val en = load("values")
        val problems = mutableListOf<String>()
        for ((lang, qs) in quantities) {
            val tr = load("values-$lang")
            (en.strings.keys - tr.strings.keys).forEach { problems += "$lang: missing string $it" }
            (tr.strings.keys - en.strings.keys).forEach { problems += "$lang: extra string $it" }
            (en.plurals.keys - tr.plurals.keys).forEach { problems += "$lang: missing plurals $it" }
            for ((name, text) in en.strings) {
                val t = tr.strings[name] ?: continue
                if (placeholders(t) != placeholders(text)) problems += "$lang: $name placeholders ${placeholders(t)}"
                if (t.isBlank()) problems += "$lang: $name is empty"
            }
            for ((name, items) in en.plurals) {
                val t = tr.plurals[name] ?: continue
                if (t.keys != qs) problems += "$lang: $name quantities ${t.keys}, want $qs"
                val other = placeholders(items.getValue("other"))
                t.forEach { (q, text) ->
                    val p = placeholders(text)
                    if (q == "other" && p != other) problems += "$lang: $name/other placeholders $p"
                    if (q != "other" && !other.containsAll(p)) problems += "$lang: $name/$q placeholders $p"
                }
            }
        }
        assertTrue(problems.joinToString("\n"), problems.isEmpty())
    }

    @Test
    fun localeConfigListsEveryLanguage() {
        val text = File(res, "xml/locales_config.xml").readText()
        for (lang in quantities.keys + "en") {
            val tag = lang.replace("-r", "-")
            assertTrue("locales_config.xml lacks $tag", text.contains("\"$tag\""))
        }
    }
}
