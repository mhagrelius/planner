// The app model, exposed to QML as the `App` singleton.
//
// Every piece of state the surfaces need lives here — which view, where the
// cursor is, what is selected, which prompt is open — and every derived
// figure is recomputed into pre-formatted rows after each change, then one
// `changed()` fires. QML is a view over these properties and calls the
// invokables when the user does something; nothing in QML touches the store.
//
// The store is canonical and this is the only thing that mutates or saves it.
// Saving is coalesced on a two-second tick so typing never blocks on I/O.
#pragma once

#include "present.h"
#include "schedule.h"
#include "store.h"
#include "sync.h"

#include <QMutex>

#include <QObject>
#include <QQmlEngine>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>

class Palette;

class App : public QObject {
    Q_OBJECT
    QML_NAMED_ELEMENT(App)
    QML_SINGLETON
    Q_PROPERTY(QString today READ today NOTIFY changed)
    Q_PROPERTY(QVariantList rail READ rail NOTIFY changed)
    Q_PROPERTY(bool railVisible READ railVisible NOTIFY changed)
    Q_PROPERTY(QString summonKey READ summonKey NOTIFY changed)
    Q_PROPERTY(QString viewId READ viewId NOTIFY changed)
    Q_PROPERTY(QString viewTitle READ viewTitle NOTIFY changed)
    Q_PROPERTY(QString viewSubtitle READ viewSubtitle NOTIFY changed)
    Q_PROPERTY(bool isProject READ isProject NOTIFY changed)
    Q_PROPERTY(bool board READ board NOTIFY changed)
    Q_PROPERTY(QVariantList lanes READ lanes NOTIFY changed)
    Q_PROPERTY(int rowCount READ rowCount NOTIFY changed)
    Q_PROPERTY(int cursor READ cursor NOTIFY changed)
    Q_PROPERTY(int boardColumn READ boardColumn NOTIFY changed)
    Q_PROPERTY(QVariantMap empty READ empty NOTIFY changed)
    Q_PROPERTY(bool selecting READ selecting NOTIFY changed)
    Q_PROPERTY(int selectionCount READ selectionCount NOTIFY changed)
    Q_PROPERTY(QString openTask READ openTask NOTIFY changed)
    Q_PROPERTY(QVariantMap detail READ detail NOTIFY changed)
    Q_PROPERTY(QString prompt READ prompt NOTIFY changed)
    Q_PROPERTY(QString promptQuery READ promptQuery NOTIFY changed)
    Q_PROPERTY(QString promptTitle READ promptTitle NOTIFY changed)
    Q_PROPERTY(QString promptPlaceholder READ promptPlaceholder NOTIFY changed)
    Q_PROPERTY(QString promptError READ promptError NOTIFY changed)
    Q_PROPERTY(int promptIndex READ promptIndex NOTIFY changed)
    Q_PROPERTY(QVariantList promptResults READ promptResults NOTIFY changed)
    Q_PROPERTY(QString promptCount READ promptCount NOTIFY changed)
    Q_PROPERTY(QVariantMap addPreview READ addPreview NOTIFY changed)
    Q_PROPERTY(bool keepAdding READ keepAdding NOTIFY changed)
    Q_PROPERTY(QVariantMap picker READ picker NOTIFY changed)
    Q_PROPERTY(QVariantList headerHints READ headerHints NOTIFY changed)
    Q_PROPERTY(QVariantMap status READ status NOTIFY changed)
    Q_PROPERTY(QString saveError READ saveError NOTIFY changed)
    Q_PROPERTY(QVariantList promptRows READ promptRows NOTIFY changed)
    Q_PROPERTY(bool syncConfigured READ syncConfigured NOTIFY changed)

public:
    struct Options {
        QString dataPath;      // the planner.json to use; empty = default
        bool demo = false;
        QDate today;           // pinned clock for grabs and tests; invalid = real
        QString screen;        // initial view id
        QStringList acts;      // states to enter before a grab
    };

    explicit App(Palette *palette, const Options &options, QObject *parent = nullptr);
    ~App() override;
    static App *create(QQmlEngine *, QJSEngine *);
    static App *instance() { return s_instance; }

    // Runs an agent command against the live store, saving immediately so the
    // caller is told the truth. Returns what to print and whether it worked.
    std::pair<QString, bool> agentCommand(const QStringList &args);
    // `planner sync now|status` from a second launch, answered by the window.
    std::pair<QString, bool> syncCommand(const QStringList &args);
    // What syncing has and has not done, as JSON for the CLI and rows for the pane.
    QJsonObject syncStatusJson() const;
    // Whether a worker is out talking to the server.
    bool syncBusy() const { return m_syncing; }
    // The tick, the reminder tick, and the final flush.
    void saveNow();

    QString today() const { return planner::dateSerial(todayDate()); }
    QVariantList rail() const { return m_rail; }
    bool railVisible() const { return m_railVisible; }
    QString summonKey() const { return m_summonKey; }
    QString viewId() const { return m_viewId; }
    QString viewTitle() const { return m_viewTitle; }
    QString viewSubtitle() const { return m_viewSubtitle; }
    bool isProject() const { return m_isProject; }
    bool board() const { return m_board; }
    QVariantList lanes() const { return m_lanes; }
    int rowCount() const { return static_cast<int>(m_rowIds.size()); }
    int cursor() const { return m_cursor; }
    int boardColumn() const { return m_boardColumn; }
    QVariantMap empty() const { return m_empty; }
    bool selecting() const { return m_selecting; }
    int selectionCount() const { return static_cast<int>(m_selection.size()); }
    QString openTask() const { return m_openTask; }
    QVariantMap detail() const { return m_detail; }
    QString prompt() const { return m_prompt; }
    QString promptQuery() const { return m_promptQuery; }
    QString promptTitle() const { return m_promptTitle; }
    QString promptPlaceholder() const { return m_promptPlaceholder; }
    QString promptError() const { return m_promptError; }
    int promptIndex() const { return m_promptIndex; }
    QVariantList promptResults() const { return m_promptResults; }
    QString promptCount() const { return m_promptCount; }
    QVariantMap addPreview() const { return m_addPreview; }
    bool keepAdding() const { return m_keepAdding; }
    QVariantMap picker() const { return m_pickerMap; }
    QVariantList headerHints() const { return m_headerHints; }
    QVariantMap status() const { return m_status; }
    QString saveError() const { return m_saveError; }
    QVariantList promptRows() const { return m_promptRows; }
    bool syncConfigured() const { return m_syncTarget.has_value(); }

    // --- sync
    Q_INVOKABLE void syncNow();
    Q_INVOKABLE void showSyncStatus();

    // --- navigation
    Q_INVOKABLE void go(const QString &viewId);
    Q_INVOKABLE void goToKey(int key);
    Q_INVOKABLE void toggleRail();
    Q_INVOKABLE void moveCursor(int delta);
    Q_INVOKABLE void cursorTo(int flat);
    Q_INVOKABLE void moveColumn(int delta);
    // The Escape cascade: picker, prompt, selection, detail. False means
    // nothing was open, so the window should close.
    Q_INVOKABLE bool escape();

    // --- the cursor row
    Q_INVOKABLE void space();           // complete/uncomplete, or toggle a selection mark
    Q_INVOKABLE void toggleTask(const QString &id);
    Q_INVOKABLE void enter();           // open the cursor row
    Q_INVOKABLE void openTaskId(const QString &id);
    Q_INVOKABLE void closeDetail();
    Q_INVOKABLE void pinCursor();
    Q_INVOKABLE void deleteKey();       // the cursor row, or the selection
    Q_INVOKABLE void moveTaskVertical(int delta);
    Q_INVOKABLE void moveTaskColumn(int delta);

    // --- selection and bulk actions
    Q_INVOKABLE void selectAll();
    Q_INVOKABLE void clearSelection();
    Q_INVOKABLE void toggleSelected(const QString &id);
    Q_INVOKABLE void setPriorityKey(int level);   // Ctrl+1–4
    Q_INVOKABLE void undo();

    // --- the detail pane
    Q_INVOKABLE void setTitle(const QString &id, const QString &text);
    Q_INVOKABLE void setDescription(const QString &id, const QString &text);
    Q_INVOKABLE void cyclePriority(const QString &id);
    Q_INVOKABLE void setPriority(const QString &id, const QString &token);
    Q_INVOKABLE void toggleLabel(const QString &id, const QString &name, bool on);
    Q_INVOKABLE void addSubtask(const QString &parent, const QString &line);
    Q_INVOKABLE void addNote(const QString &id, const QString &text);
    Q_INVOKABLE void removeNote(const QString &id, const QString &at);
    Q_INVOKABLE void openDeadlinePicker();

    // --- prompts: palette, quick add, quick find, text input, confirmation
    Q_INVOKABLE void openPrompt(const QString &kind);
    Q_INVOKABLE void closePrompt();
    Q_INVOKABLE void setPromptQuery(const QString &text);
    Q_INVOKABLE void movePrompt(int delta);
    Q_INVOKABLE void promptTo(int index);
    Q_INVOKABLE void runPrompt();
    Q_INVOKABLE void submitKeepAdding();

    // --- project structure
    Q_INVOKABLE void toggleStyle();
    Q_INVOKABLE void newSection();
    Q_INVOKABLE void newProject();

    // --- the date picker
    Q_INVOKABLE void openDatePicker();
    Q_INVOKABLE void pickerType(const QString &text);
    Q_INVOKABLE void pickerQuick(const QString &which);
    Q_INVOKABLE void pickerMonth(int delta);
    Q_INVOKABLE void pickerDay(const QString &iso);
    Q_INVOKABLE void pickerTime(const QString &text);
    Q_INVOKABLE void pickerRepeat(const QString &text);
    Q_INVOKABLE void pickerClearRepeat();
    Q_INVOKABLE void closePicker();

    // --- testing hooks
    Q_INVOKABLE void act(const QString &name);

signals:
    void changed();
    void windowRequested();

private:
    QDate todayDate() const;
    QDateTime now() const;
    template <typename F> void mutate(F change);
    void recompute();
    void buildRail();
    void buildContent();
    void buildDetail();
    void buildPrompt();
    void buildPicker();
    void buildStatus();
    QVariantMap row(const planner::Task &task, int lane, int index, int flat) const;
    std::optional<planner::View> currentView() const;
    std::optional<planner::TaskId> cursorId() const;
    QList<planner::TaskId> targets() const;   // the selection, or the cursor row
    void toast(const QString &text, bool undoable);
    void completeIds(const QList<planner::TaskId> &ids);
    void deleteIds(const QList<planner::TaskId> &ids);
    void applyPicker();
    void runPaletteItem(const QVariantMap &item);
    void beginInput(const QString &action, const QString &title, const QString &placeholder, const QString &prefill, const QVariantMap &payload);
    void finishInput();
    void fireReminders();
    void notify(const QString &title, const QString &body);
    void readSummonKey();
    void startSync();
    void finishSync(const std::optional<planner::sync::Incoming> &incoming, const QString &error);
    void waitForChanges();
    void finishWait(bool ok, bool changed, const QDateTime &cursor);
    void syncAfterEdit();
    void followCursor();
    void reportSyncFailure(const QString &message);
    template <typename F> static void onMainThread(F functor);

    static App *s_instance;
    static QMutex s_instanceMutex;   // workers check the instance is still here
    Palette *m_palette;
    planner::Store m_store;
    planner::LoadOutcome m_loadOutcome;
    planner::Schedule m_schedule;
    bool m_dirty = false;
    QString m_saveError;
    QTimer m_saveTick;
    QTimer m_reminderTick;
    QTimer m_toastTimer;
    QDate m_pinnedToday;

    // Sync: where to, what was agreed, and how it has been going.
    std::optional<std::pair<QString, QString>> m_syncTarget;
    planner::sync::Snapshot m_syncBase;
    bool m_syncing = false;
    bool m_applyingSync = false;
    int m_syncFailures = 0;
    QString m_syncError;         // shown once failures have persisted
    QString m_syncLastFailure;
    std::optional<QDateTime> m_syncLastPass;
    std::optional<QDateTime> m_syncCursor;
    QTimer m_syncTick;
    QTimer m_syncSoon;
    QVariantList m_promptRows;

    // Per-surface state, everything else is derived.
    QString m_viewId = QStringLiteral("today");
    bool m_railVisible = true;
    QString m_summonKey;
    int m_cursor = 0;
    int m_boardColumn = 0;
    QString m_cursorWanted;             // the id the cursor was on, kept across a refresh
    bool m_selecting = false;
    QStringList m_selection;
    QString m_openTask;
    QString m_prompt;                   // "", palette, add, find, input, confirm
    QString m_promptQuery;
    QString m_promptTitle, m_promptPlaceholder, m_promptError;
    int m_promptIndex = 0;
    bool m_keepAdding = false;
    QString m_inputAction;
    QVariantMap m_inputPayload;
    struct Picker {
        bool open = false;
        QString mode;                   // due, deadline, bulk
        QStringList tasks;
        QDate month;
        std::optional<QDate> date;
        QString time;
        QString repeat;
        QString repeatError;
        bool hadDate = false;
        std::optional<planner::Recurrence> rule;
    } m_picker;
    struct Undo {
        QString kind;                   // completed, deleted, section, project
        QList<planner::TaskId> ids;
        QList<planner::Task> tasks;
        std::optional<planner::RemovedSection> section;
        std::optional<planner::RemovedProject> project;
    };
    std::optional<Undo> m_undo;
    QVariantMap m_toast;

    // Derived.
    QVariantList m_rail;
    QString m_viewTitle, m_viewSubtitle;
    bool m_isProject = false, m_board = false;
    QVariantList m_lanes;
    QList<planner::TaskId> m_rowIds;    // flat order
    QList<std::pair<int, int>> m_rowPlaces;   // lane, index per flat row
    QList<std::optional<planner::SectionId>> m_laneSections;
    QVariantMap m_empty;
    QVariantMap m_detail;
    QVariantList m_promptResults;
    QString m_promptCount;
    QVariantMap m_addPreview;
    QVariantMap m_pickerMap;
    QVariantList m_headerHints;
    QVariantMap m_status;
};
