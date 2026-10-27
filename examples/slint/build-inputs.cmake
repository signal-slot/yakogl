# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Run by CMakeLists.txt as `cmake -P` at every build (not at configure time, so
# an edit of the trees outside the repository shows in the next image).
# Writes ${OUT}, the build inputs of the Slint staticlib the image links:
#
#   slint <git HEAD, 8>+<hash of the tree, 12> femtovg <version>+<hash, 12>
#   glow <version>+<hash, 12> lock <hash, 12 | unpinned>
#
# rust/build.rs adds `rustc <release (commit date)>` and compiles the line
# into the image; src/main.c prints it as "slint-build: ..." at boot.
#
# It also enforces the one pinned input that is not in a lock file: the Slint
# tree must be the checkout nostd/README.md names (SLINT_BASE_REV) with the
# patch applied. A tree with a git history at another commit stops the build.
#
# Inputs (-D): SLINT_NOSTD_DIR NOSTD_PATCH_DIR SLINT_BASE_REV LOCK_FILE
#              LOCKED (ON: cargo runs --locked) OUT [GIT_EXECUTABLE]

# The hash of the sources under the given roots: SHA-256 over "path hash" lines.
function(tree_hash out root)
  set(files "")
  foreach(dir IN LISTS ARGN)
    file(GLOB_RECURSE found "${root}/${dir}/*.rs" "${root}/${dir}/*.toml"
         "${root}/${dir}/*.slint" "${root}/${dir}/*.glsl")
    list(APPEND files ${found})
  endforeach()
  list(FILTER files EXCLUDE REGEX "/(target|tests)/")
  list(SORT files)
  set(all "")
  foreach(f IN LISTS files)
    file(RELATIVE_PATH rel "${root}" "${f}")
    file(SHA256 "${f}" h)
    string(APPEND all "${rel} ${h}\n")
  endforeach()
  string(SHA256 digest "${all}")
  string(SUBSTRING "${digest}" 0 12 digest)
  set(${out} "${digest}" PARENT_SCOPE)
endfunction()

# `version = "x"` of a crate's [package] table.
function(crate_version out toml)
  file(STRINGS "${toml}" line REGEX "^version = \"" LIMIT_COUNT 1)
  string(REGEX REPLACE "^version = \"([^\"]*)\".*" "\\1" v "${line}")
  set(${out} "${v}" PARENT_SCOPE)
endfunction()

# Slint: git HEAD when the tree is a checkout, and the base check.
set(head "nogit")
if(EXISTS "${SLINT_NOSTD_DIR}/.git" AND GIT_EXECUTABLE)
  execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${SLINT_NOSTD_DIR}" rev-parse HEAD
                  OUTPUT_VARIABLE rev OUTPUT_STRIP_TRAILING_WHITESPACE RESULT_VARIABLE rc
                  ERROR_QUIET)
  if(rc EQUAL 0)
    if(NOT rev STREQUAL SLINT_BASE_REV)
      message(FATAL_ERROR
        "slint: SLINT_NOSTD_DIR='${SLINT_NOSTD_DIR}' is at ${rev}, not at ${SLINT_BASE_REV}, "
        "the revision nostd/README.md patches. Recreate the tree as it says. The Pi 4 picture "
        "changed between images that nothing in the repository explains (issue #1284); "
        "the base of the Slint tree is pinned for that.")
    endif()
    string(SUBSTRING "${rev}" 0 8 head)
  else()
    set(head "unknown")
  endif()
endif()
tree_hash(slint_hash "${SLINT_NOSTD_DIR}" internal api/rs)

crate_version(fvg_ver "${NOSTD_PATCH_DIR}/femtovg/Cargo.toml")
crate_version(glow_ver "${NOSTD_PATCH_DIR}/glow/Cargo.toml")
tree_hash(fvg_hash "${NOSTD_PATCH_DIR}" femtovg)
tree_hash(glow_hash "${NOSTD_PATCH_DIR}" glow)

if(LOCKED)
  file(SHA256 "${LOCK_FILE}" lock)
  string(SUBSTRING "${lock}" 0 12 lock)
else()
  set(lock "unpinned")
endif()

file(WRITE "${OUT}.tmp"
  "slint ${head}+${slint_hash} femtovg ${fvg_ver}+${fvg_hash} glow ${glow_ver}+${glow_hash} lock ${lock}")
# Replace only on a change, so cargo does not rebuild for an unchanged line.
execute_process(COMMAND "${CMAKE_COMMAND}" -E copy_if_different "${OUT}.tmp" "${OUT}")
file(REMOVE "${OUT}.tmp")
