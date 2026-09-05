cmake_minimum_required(VERSION 3.21)
project(planner VERSION 0.2.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 DBus Svg Network Test)
qt_standard_project_setup(REQUIRES 6.5)
qt_policy(SET QTP0004 NEW)  # .qml files in subdirectories without a qmldir

# ---- core: plain C++ (QtCore only), headless, unit-tested -------------------
qt_add_library(plannercore STATIC
    src/core/model.cpp
    src/core/dates.cpp
    src/core/quickadd.cpp
    src/core/store.cpp
    src/core/query.cpp
    src/core/search.cpp
    src/core/schedule.cpp
    src/core/present.cpp
    src/core/agent.cpp
    src/core/demo.cpp
    src/core/order.cpp
    src/core/sync.cpp
    src/core/config.cpp
)
target_include_directories(plannercore PUBLIC src/core)
target_link_libraries(plannercore PUBLIC Qt6::Core)

# ---- app: the App singleton and the QML views ------------------------------
option(PLANNER_APP "Build the Qt Quick app (off for core-only test builds)" ON)
if(PLANNER_APP)
qt_add_executable(planner
    src/app/main.cpp
    src/app/systemtheme.cpp
    src/app/palette.cpp
    src/app/app.cpp
    src/app/icons.cpp
    src/app/single.cpp
    src/app/remote.cpp
)

set_source_files_properties(src/app/qml/T.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

file(GLOB PLANNER_QML RELATIVE ${CMAKE_SOURCE_DIR} CONFIGURE_DEPENDS
    src/app/qml/*.qml
    src/app/qml/components/*.qml
    src/app/qml/views/*.qml)
file(GLOB PLANNER_ICONS RELATIVE ${CMAKE_SOURCE_DIR} CONFIGURE_DEPENDS src/app/icons/*.svg)

# Keep the default resource prefix (qrc:/qt/qml/<URI>): the module must be
# found from the installed binary, not only from the build tree.
qt_add_qml_module(planner
    URI Planner
    VERSION 1.0
    SOURCES
        src/app/palette.h
        src/app/app.h
    QML_FILES ${PLANNER_QML}
    RESOURCES ${PLANNER_ICONS}
)

target_include_directories(planner PRIVATE src src/app)
target_link_libraries(planner PRIVATE plannercore
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::DBus Qt6::Svg Qt6::Network)
endif()

# ---- tests ------------------------------------------------------------------
enable_testing()
foreach(t dates recurrence quickadd query store search schedule present agent order sync)
    qt_add_executable(test_${t} tests/test_${t}.cpp)
    target_link_libraries(test_${t} PRIVATE plannercore Qt6::Test)
    add_test(NAME ${t} COMMAND test_${t})
endforeach()
# The single-instance socket is app code, tested on its own with a fake window.
qt_add_executable(test_single tests/test_single.cpp src/app/single.cpp)
target_include_directories(test_single PRIVATE src/app)
target_link_libraries(test_single PRIVATE plannercore Qt6::Network Qt6::Test)
add_test(NAME single COMMAND test_single)

include(GNUInstallDirs)
if(PLANNER_APP)
install(TARGETS planner RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES packaging/planner.desktop DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(FILES packaging/planner.svg DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps)
endif()
