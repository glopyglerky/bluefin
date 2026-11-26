package org.bluefin.android

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation

/** TV focus remains navigable until Select explicitly enters native editing. */
@Composable internal fun TextEntry(
    value: String, change: (String) -> Unit, label: String, television: Boolean,
    modifier: Modifier = Modifier, next: FocusRequester? = null,
    previous: FocusRequester? = null, imeAction: ImeAction = ImeAction.Done,
    keyboardType: KeyboardType = KeyboardType.Text, secret: Boolean = false
) {
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    var editing by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    var sawKeyboard by remember { mutableStateOf(false) }
    val keyboardVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    fun finish() { keyboard?.hide(); editing = false }
    fun nextField() {
        finish()
        if (next != null) next.requestFocus() else focus.moveFocus(FocusDirection.Down)
    }
    LaunchedEffect(editing) {
        if (editing) { withFrameNanos { }; keyboard?.show() }
    }
    LaunchedEffect(editing, keyboardVisible) {
        if (!editing) sawKeyboard = false
        else if (keyboardVisible) sawKeyboard = true
        else if (sawKeyboard) { editing = false; sawKeyboard = false }
    }
    // Android dismisses the IME before dispatching Back to the screen.
    // A second local BackHandler would consume the following navigation Back.
    val navigation = if (!television) modifier else modifier
        .onFocusChanged { focused = it.isFocused; if (!it.isFocused) editing = false }
        .onPreviewKeyEvent { event ->
            when (event.key) {
                Key.DirectionDown -> {
                    if (event.type == KeyEventType.KeyDown) nextField()
                    true
                }
                Key.DirectionUp -> {
                    if (event.type == KeyEventType.KeyDown) {
                        finish()
                        if (previous != null) previous.requestFocus() else focus.moveFocus(FocusDirection.Up)
                    }
                    true
                }
                Key.DirectionCenter, Key.Enter, Key.NumPadEnter -> {
                    if (event.type == KeyEventType.KeyDown) {
                        if (editing) keyboard?.show() else editing = true
                    }
                    true
                }
                else -> false
            }
        }
    OutlinedTextField(value, change, label = { Text(label) }, singleLine = true,
        modifier = navigation, readOnly = television && !editing,
        visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = if (television) KeyboardOptions(imeAction = imeAction, keyboardType = keyboardType) else KeyboardOptions.Default,
        keyboardActions = if (television) KeyboardActions(onNext = { nextField() }, onDone = { finish() }) else KeyboardActions.Default)
}
