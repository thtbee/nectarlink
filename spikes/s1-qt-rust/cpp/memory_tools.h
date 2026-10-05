// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtCore/QObject>
#include <QtQml/QQmlEngine>
#include <QtQml/qqmlregistration.h>

// Releases memory the QML engine keeps after windows are destroyed.
class MemoryTools : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON

public:
    using QObject::QObject;

    // Collects garbage and drops compiled components that have no live
    // instances (the main window's types after it is closed).
    Q_INVOKABLE void trimEngineCaches()
    {
        if (QQmlEngine *engine = qmlEngine(this)) {
            engine->collectGarbage();
            engine->trimComponentCache();
        }
    }
};
