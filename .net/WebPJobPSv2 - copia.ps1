#*************** NECESITA POWERSHELL 7!! ***************

# Parámetros y variables de configuración
$zipExe      = "C:\Program Files\7-Zip\7z.exe"
$folder      = 'C:\Users\josed\Downloads\x'         # Parámetro 1. (sin \ final)
$folderTarget= 'C:\temp\x\manga2\'        # Parámetro 2. (con \ final)
$threads     = 8                    # Parámetro 3.
$finalFolder = "E:\manga"           # Parámetro 4. (destino final de los CBZ)
$resize      = '100%'               # Parámetro 5. tamaño
$quality     = 67                   # Parámetro 6. calidad
$borrarSource= $false                # Parámetro 7. (True = borra si el tamaño final es menor)
$copiarFinal = $true                # Parámetro 8. (True = mover zip final a $finalFolder)
$folderToIgnore = $folder

# NUEVA VARIABLE: El modo de zipeo. # parametro 9.
# Asigna "single" para comprimir todo en un único archivo CBZ,
# o "individual" para generar un archivo CBZ por cada elemento original (archivo o carpeta).
$zipMode = "single"    # Opciones: "single" o "individual"

# Inicio del proceso
Clear-Host
Set-Location $folder
$inicio = Get-Date

# Calcula el tamaño inicial (en MB)
$tamañoIni = (Get-ChildItem -Path $folder -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB

# Paso 1. Extraer archivos comprimidos encontrados (zip, 7z, cbz, cbr, rar)
$filesToExtract = Get-ChildItem -Path $folder -Recurse -Include *.zip, *.7z, *.cbz, *.cbr, *.rar
foreach ($file in $filesToExtract) {
    # Se crea una carpeta de destino con el mismo nombre base que el archivo
    $destinationFolder = Join-Path $folder ($file.BaseName)
    # Se extrae el contenido usando 7-Zip
    & $zipExe x "$($file.FullName)" -o"$destinationFolder"
}
Write-Host "Extraction complete."

# Paso 2. Eliminar los archivos comprimidos ya extraídos
foreach ($file in $filesToExtract) {
    Remove-Item -Path $file
}

# Paso 3. Crear la estructura de carpetas en $folderTarget
New-Item -Path $folderTarget -ItemType "directory" -Force | Out-Null
Get-ChildItem -Recurse -Directory | ForEach-Object {
    $newDir = $_.FullName.Replace($folderToIgnore, $folderTarget)
    New-Item -Path $newDir -ItemType "directory" -Force | Out-Null
    # Para depuración, se pueden imprimir las rutas:
    # Write-Host "Creada: $newDir"
}

# Paso 4. Conversión de imágenes a webp (en paralelo)
$allFiles = Get-ChildItem -Recurse -File
$total    = $allFiles.Count

if ($total -eq 0) {
    Write-Host "No hay archivos para convertir."
} else {
    $allFiles |
        ForEach-Object -Parallel {
            # Se calcula el directorio de destino para el archivo actual
            $destDir = "$($using:folderTarget)$($_.DirectoryName.Replace($using:folder,''))"

            if ($using:resize -eq '100%') {
                magick.exe mogrify -trim -define trim:minSize='75%' +repage -path $destDir -quality $using:quality -format webp $_
            }
            else {
                magick.exe mogrify -trim -define trim:minSize='75%' +repage -resize $using:resize -path $destDir -quality $using:quality -format webp $_
            }

            # Emitir un marcador por cada archivo completado (para el progreso en el hilo principal)
            [PSCustomObject]@{ Path = $_.FullName }
        } -ThrottleLimit $threads |
        ForEach-Object -Begin {
            $i = 0
        } -Process {
            $i++
            $pct = if ($total -gt 0) { [int](($i / $total) * 100) } else { 100 }
            Write-Progress -Activity "Convirtiendo imágenes a webp ($threads hilos)" `
                           -Status    "$i de $total" `
                           -PercentComplete $pct
        } -End {
            Write-Progress -Activity "Convirtiendo imágenes a webp" -Completed
        }
}

# Calcula el tamaño final (en MB) de los archivos convertidos
$tamañoFin = (Get-ChildItem -Path $folderTarget -Recurse | Measure-Object -Property Length -Sum).Sum / 1MB

# Paso 5. Empaquetado en CBZ según el modo seleccionado

# Determinar dónde crear el zip final
if ($copiarFinal) {
    $destDir = $finalFolder
} else {
    $destDir = $folderTarget
}

if ($zipMode -eq "single") {
    # Modo "single": Se genera un único archivo CBZ que contiene TODO lo de $folderTarget.
    # Nuevo criterio de nombre: usar el nombre del PRIMER subelemento (carpeta o archivo)
    # encontrado directamente dentro de $folderTarget. Si no hay elementos, usar el nombre
    # de la carpeta destino (sin la barra final) como antes.
    $firstItem = Get-ChildItem -Path $folderTarget | Select-Object -First 1
    if ($null -ne $firstItem) {
        if ($firstItem.PSIsContainer) {
            $baseName = $firstItem.Name
        }
        else {
            $baseName = $firstItem.BaseName
        }
    }
    else {
        $folderTargetTrimmed = $folderTarget.TrimEnd('\\')
        $baseName = Split-Path -Path $folderTargetTrimmed -Leaf
    }
    $singleZipName = "$baseName.cbz"
    $singleZipPath = Join-Path $destDir $singleZipName
    # Nota: Se usan comillas dobles para que $folderTarget* expanda la ruta; 
    # asegúrese de que $folderTarget tenga la barra final.
    & $zipExe a -tzip -mx0 -sdel "$singleZipPath" "$folderTarget*"
}
elseif ($zipMode -eq "individual") {
    # Modo "individual": Se genera un archivo CBZ para cada elemento (archivo o carpeta) que
    # exista directamente dentro de $folderTarget.
    $items = Get-ChildItem -Path $folderTarget
    foreach ($item in $items) {
        if ($item.PSIsContainer) {
            # Si es una carpeta, se comprime todo su contenido.
            $zipName = "$($item.Name).cbz"
            $zipPath = Join-Path $destDir $zipName
            & $zipExe a -tzip -mx0 -sdel "$zipPath" "$($item.FullName)\*"
        }
        else {
            # Si es un archivo individual, se comprime ese mismo archivo.
            $zipName = "$($item.BaseName).cbz"
            $zipPath = Join-Path $destDir $zipName
            & $zipExe a -tzip -mx0 -sdel "$zipPath" "$($item.FullName)"
        }
    }
}
else {
    Write-Host "El valor de `\$zipMode ($zipMode) no es válido. Use 'single' o 'individual'."
}

# Paso 6. Si corresponde, borrar la fuente original (si el tamaño final es menor)
if ($borrarSource) {
    if ($tamañoFin -le $tamañoIni) {
        Remove-Item -Path "$folder\*" -Recurse -Force
    }
}

# Mostrar estadísticas y tiempo transcurrido
$fin = Get-Date
$ts  = (New-TimeSpan -Start $inicio -End $fin).TotalMinutes
Write-Host "Tiempo en minutos: $ts"
Write-Host "Tamaño inicial: $tamañoIni MB"
Write-Host "Tamaño final:   $tamañoFin MB"
