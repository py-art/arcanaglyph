#!/bin/bash
# arcanaglyph/install_libvosk.bash

# Останавливаемся при любой ошибке
set -e

# Создаем рабочую папку
echo "--- Preparing workspace in ~/vosk-build ---"
mkdir -p ~/vosk-build
cd ~/vosk-build

# --- Шаг 1: Клонируем Kaldi ---
echo "--- Cloning Kaldi (Vosk fork) ---"
if [ ! -d "kaldi" ]; then
	git clone -b vosk --single-branch https://github.com/alphacep/kaldi
fi

# --- Шаг 2: Собираем зависимости Kaldi ---
echo "--- Building Kaldi dependencies (this will take a while) ---"
cd kaldi/tools
make openfst cub
./extras/install_openblas_clapack.sh

# --- Шаг 3: Собираем Kaldi ---
echo "--- Building Kaldi engine (this is the longest step) ---"
cd ../src
./configure --mathlib=OPENBLAS_CLAPACK --shared
make -j "$(nproc)" online2 lm rnnlm

# --- Шаг 4: Собираем vosk-api ---
echo "--- Building vosk-api (libvosk.so) ---"
cd ~/vosk-build
if [ ! -d "vosk-api" ]; then
	git clone https://github.com/alphacep/vosk-api
fi
cd vosk-api/src
KALDI_ROOT=../../kaldi make

# --- Шаг 5: Устанавливаем библиотеку ---
echo "--- Installing libvosk.so to the system ---"
sudo make install
sudo ldconfig

echo "--- All done! libvosk.so is successfully built and installed. ---"
